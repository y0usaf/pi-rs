-- File-backed sting8k_pi-vcc translation (dogfood package).
-- VCC (Vague Compaction & Collaboration): structured-summary compaction,
-- history recall (command + tool), and before-compact hook.
-- Public surface only: pi.register_command, pi.register_tool, pi.on, pi.sendMessage,
-- ctx.compact, ctx.sessionManager.{getSessionFile,getBranch,getEntries},
-- ctx.ui.notify, pi.module.require("pi.agent.messages","1").convert_to_llm,
-- pi.fs.{exists,read_file,mkdir,write_file_atomic}, pi.path.{join,dirname},
-- pi.env.{HOME,USERPROFILE,PI_VCC_CONFIG_PATH}, pi.json.{decode,encode},
-- pi.set_timeout, pi.cwd. Cleanup: stateless besides file-backed settings and a
-- module-level lastStats pair of scalars; no host process/socket/timer outlives
-- its dispatch (the one session_compact setTimeout is dispatch-scoped).
local pi = ...

local agent_messages = pi.module.require("pi.agent.messages", "1")
local convert_to_llm = agent_messages.convert_to_llm

-- ── Settings ───────────────────────────────────────────────────────
local function get_agent_dir()
  local home = pi.env.HOME or pi.env.USERPROFILE
  if home then return pi.path.join(home, ".pi", "agent") end
  return pi.path.join(".pi", "agent")
end
local function settings_path()
  if pi.env.PI_VCC_CONFIG_PATH and pi.env.PI_VCC_CONFIG_PATH:gsub("%s", "") ~= "" then
    return pi.env.PI_VCC_CONFIG_PATH
  end
  return pi.path.join(get_agent_dir(), "pi-vcc-config.json")
end

local DEFAULT_SETTINGS = { overrideDefaultCompaction = false, debug = false }

local function load_settings()
  local path = settings_path()
  if not pi.fs.exists(path) then
    local base = {}
    for k, v in pairs(DEFAULT_SETTINGS) do base[k] = v end
    return base
  end
  local ok, raw = pcall(pi.fs.read_file, path)
  local parsed
  if ok then
    local ok2, val = pcall(pi.json.decode, raw)
    parsed = ok2 and val or nil
  end
  if type(parsed) ~= "table" then return { overrideDefaultCompaction = false, debug = false } end
  local out = {}
  for k, v in pairs(DEFAULT_SETTINGS) do
    if type(parsed[k]) == type(v) then out[k] = parsed[k] else out[k] = v end
  end
  return out
end

local function scaffold_settings()
  local ok_run = pcall(function()
    local path = settings_path()
    local dir = pi.path.dirname(path)
    if not pi.fs.exists(dir) then pi.fs.mkdir(dir, true) end
    if not pi.fs.exists(path) then
      pi.fs.write_file_atomic(path, pi.json.encode(DEFAULT_SETTINGS, true) .. "\n")
      return
    end
    local ok, raw = pcall(pi.fs.read_file, path)
    if not ok then return end
    local ok2, parsed = pcall(pi.json.decode, raw)
    if not ok2 or type(parsed) ~= "table" then return end
    local changed = false
    for key, value in pairs(DEFAULT_SETTINGS) do
      if parsed[key] == nil then
        parsed[key] = value
        changed = true
      end
    end
    if changed then pi.fs.write_file_atomic(path, pi.json.encode(parsed, true) .. "\n") end
  end)
  if not ok_run then end
end

-- ── Content utils ──────────────────────────────────────────────────
local function clip(text, max)
  max = max or 200
  text = tostring(text or "")
  if #text <= max then return text end
  local last_space = text:match("^.*()%s.*$", 1)
  local end_pos
  if last_space and last_space > max * 0.6 then end_pos = last_space - 1 else end_pos = max end
  if end_pos < 0 then end_pos = 0 end
  return text:sub(1, end_pos)
end

local function clip_sentence(text, max)
  max = max or 200
  text = tostring(text or "")
  if #text <= max then return text end
  local window = text:sub(1, max)
  local last = nil
  for m in window:gmatch("().[%.!?][ \n]") do last = m end
  for m in window:gmatch(".[%.!?]()") do last = m end
  if last and last >= max * 0.5 then return text:sub(1, last) end
  return clip(text, max)
end

local function non_empty_lines(text)
  local out = {}
  for line in tostring(text or ""):gmatch("[^\n]+") do
    local t = line:gsub("^%s+", ""):gsub("%s+$", "")
    if t ~= "" then out[#out + 1] = t end
  end
  return out
end

local function first_line(text, max)
  local line = tostring(text or ""):match("^[^\n]*") or ""
  return clip(line, max or 200)
end

local function text_of(content)
  local parts = {}
  if type(content) == "string" then return content end
  if type(content) == "table" then
    for i = 1, #content do
      local part = content[i]
      if type(part) == "table" and part.type == "text" and type(part.text) == "string" then
        parts[#parts + 1] = part.text
      end
    end
  end
  return table.concat(parts, "\n")
end

local function snippet(text, term, radius)
  radius = radius or 60
  local lowered = text:lower()
  local idx = lowered:find(term:lower(), 1, true)
  if not idx then return nil end
  local start = math.max(0, idx - 1 - radius)
  local endp = math.min(#text, idx - 1 + #term + radius)
  local prefix = (start > 0) and "..." or ""
  local suffix = (endp < #text) and "..." or ""
  return prefix .. text:sub(start + 1, endp) .. suffix
end

-- ── Sanitize ───────────────────────────────────────────────────────
local function sanitize(text)
  text = tostring(text or "")
  text = text:gsub("\r\n", "\n"):gsub("\r", "\n")
  text = text:gsub("\27%[[0-9;]*[%a]", "")
  text = text:gsub("[\0-\8\11\12\14-\31]", "")
  return text
end

-- ── Tool args ──────────────────────────────────────────────────────
local function extract_path(args)
  if type(args) ~= "table" then return nil end
  for _, key in ipairs({ "path", "file_path", "filePath", "file" }) do
    if type(args[key]) == "string" then return args[key] end
  end
  return nil
end

local function summarize_tool_args(args)
  if type(args) ~= "table" then return "" end
  local path = extract_path(args)
  if path then return "path=" .. path end
  if type(args.command) == "string" then return "command=" .. args.command end
  if type(args.query) == "string" then return "query=" .. args.query end
  local keys = {}
  for k in pairs(args) do keys[#keys + 1] = tostring(k) end
  return table.concat(keys, ", ")
end

-- ── Render entries ─────────────────────────────────────────────────
local function tool_calls_text(content)
  if type(content) ~= "table" then return "" end
  local parts = {}
  for i = 1, #content do
    local c = content[i]
    if type(c) == "table" and c.type == "toolCall" then
      parts[#parts + 1] = tostring(c.name) .. "(" .. summarize_tool_args(c.arguments) .. ")"
    end
  end
  return table.concat(parts, ", ")
end

local function extract_files_from_content(content)
  local out = {}
  if type(content) ~= "table" then return out end
  for i = 1, #content do
    local c = content[i]
    if type(c) == "table" and c.type == "toolCall" then
      local p = extract_path(c.arguments)
      if p then out[#out + 1] = p end
    end
  end
  return out
end

local function render_message(msg, index, full)
  full = full == true
  if not (type(msg) == "table") then return { index = index, role = "unknown", summary = "" } end
  if msg.role == "user" then
    return { index = index, role = "user", summary = full and text_of(msg.content) or clip(text_of(msg.content), 300) }
  end
  if msg.role == "toolResult" then
    local prefix = msg.isError and "ERROR " or ""
    local text = full and text_of(msg.content) or clip(text_of(msg.content), 200)
    return { index = index, role = "tool_result", summary = prefix .. "[" .. tostring(msg.toolName) .. "] " .. text }
  end
  if msg.role == "bashExecution" then
    local cmd = tostring(msg.command or "")
    local out = tostring(msg.output or "")
    local text = full and ("$ " .. cmd .. "\n" .. out) or clip("$ " .. cmd .. "\n" .. out, 300)
    return { index = index, role = "bash", summary = text }
  end
  local text = full and text_of(msg.content) or clip(text_of(msg.content), 300)
  local tools = tool_calls_text(msg.content)
  local files = extract_files_from_content(msg.content)
  local summary = (tools ~= "") and (tools .. "\n" .. text) or text
  local out = { index = index, role = "assistant", summary = summary }
  if #files > 0 then out.files = files end
  return out
end

-- ── Load messages (session JSONL) ──────────────────────────────────
local function load_all_messages(session_file, full, allowed_entry_ids)
  local rendered, raw_messages, entry_ids = {}, {}, {}
  local ok_read, content = pcall(pi.fs.read_file, session_file)
  if not ok_read then
    return { rendered = rendered, rawMessages = raw_messages, entryIds = entry_ids }
  end
  local message_index = 0
  for line in content:gmatch("[^\n]+") do
    if line:gsub("%s", "") ~= "" then
      local ok, e = pcall(pi.json.decode, line)
      if ok and type(e) == "table" and e.type == "message" and e.message then
        local allowed = (not allowed_entry_ids) or allowed_entry_ids[e.id] == true
        if allowed then
          rendered[#rendered + 1] = render_message(e.message, message_index, full)
          raw_messages[#raw_messages + 1] = e.message
          entry_ids[#entry_ids + 1] = tostring(e.id)
        end
        message_index = message_index + 1
      end
    end
  end
  return { rendered = rendered, rawMessages = raw_messages, entryIds = entry_ids }
end

-- ── Lineage / scope ────────────────────────────────────────────────
local function get_active_lineage_entry_ids(session_manager)
  local ids = {}
  local ok, branch = pcall(function() return session_manager.getBranch() end)
  if ok and type(branch) == "table" then
    for _, entry in ipairs(branch) do
      if type(entry) == "table" and type(entry.id) == "string" and entry.id ~= "" then ids[entry.id] = true end
    end
    local has = false
    for _ in pairs(ids) do has = true break end
    if has then return ids end
  end
  local ok2, all = pcall(function() return session_manager.getEntries() end)
  if ok2 and type(all) == "table" then
    for _, entry in ipairs(all) do
      if type(entry) == "table" and type(entry.id) == "string" and entry.id ~= "" then ids[entry.id] = true end
    end
  end
  return ids
end

local function normalize_recall_scope(scope)
  if type(scope) == "string" and scope:lower() == "all" then return "all" end
  return "lineage"
end

local function parse_recall_scope(text)
  local scope = "lineage"
  local cleaned = text:gsub("%bscope:(lineage|all)%b", function()
    return ""
  end)
  cleaned = text
  local lowered = text:lower()
  if lowered:match("scope:%s*all") then scope = "all" end
  cleaned = cleaned:gsub("[Ss][Cc][Oo][Pp][Ee]:%s*[Aa][Ll][Ll]", ""):gsub("[Ss][Cc][Oo][Pp][Ee]:%s*[Ll][Ii][Nn][Ee][Aa][Gg][Ee]", "")
  cleaned = cleaned:gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
  return { scope = scope, text = cleaned }
end

-- ── Skill collapse ─────────────────────────────────────────────────
local function collapse_skill_lines(lines)
  local result = {}
  local seen = {}
  local inside = false
  for _, line in ipairs(lines) do
    local name = line:match("^%-?%s*<skill%s+name=\"([^\"]+)\"")
    if name then
      inside = true
      if not seen[name] then
        seen[name] = true
        result[#result + 1] = "[skill: " .. name .. "]"
      end
    elseif inside then
      if line:gsub("^%-?%s*</skill>%s*$", "") == "" then inside = false end
    else
      result[#result + 1] = line
    end
  end
  return result
end

local function collapse_skill_text(text)
  return (tostring(text or ""):gsub("<skill%s+name=\"([^\"]+)\"[^>]*>.-(</skill>|$)", function(name) return "[skill: " .. name .. "]" end))
end

-- ── Filter noise ───────────────────────────────────────────────────
local NOISE_TOOLS = {}
for _, n in ipairs({ "TodoWrite", "TodoRead", "ToolSearch", "WebSearch", "AskUser", "ExitSpecMode", "GenerateDroid" }) do NOISE_TOOLS[n] = true end
local NOISE_STRINGS = { "Continue from where you left off.", "No response requested.", "IMPORTANT: TodoWrite was not called yet." }

local function strip_xml_wrappers(text)
  local out = tostring(text or "")
  out = out:gsub("<(system%-reminder|ide_opened_file|command%-message|context%-window%-usage)[^>]*>.-</%1>", "")
  return out
end

local function is_noise_user_block(text)
  local trimmed = tostring(text or ""):gsub("^%s+", ""):gsub("%s+$", "")
  for _, n in ipairs(NOISE_STRINGS) do
    if trimmed:find(n, 1, true) then return true end
  end
  return strip_xml_wrappers(trimmed):gsub("%s", "") == ""
end

local function clean_user_text(text)
  return strip_xml_wrappers(text):gsub("^%s+", ""):gsub("%s+$", "")
end

local function filter_noise(blocks)
  local out = {}
  for _, b in ipairs(blocks) do
    if b.kind == "thinking" then
      -- skip
    elseif b.kind == "tool_call" and NOISE_TOOLS[b.name] then
    elseif b.kind == "tool_result" and NOISE_TOOLS[b.name] then
    elseif b.kind == "user" then
      if is_noise_user_block(b.text) then
        -- skip
      else
        local cleaned = clean_user_text(b.text)
        if cleaned ~= "" then out[#out + 1] = { kind = "user", text = cleaned } end
      end
    else
      out[#out + 1] = b
    end
  end
  return out
end

-- ── Normalize ──────────────────────────────────────────────────────
local function normalize_one(msg, msg_index)
  if type(msg) ~= "table" then return {} end
  if msg.role == "user" then
    local blocks = {}
    local text = sanitize(text_of(msg.content))
    if text ~= "" then blocks[#blocks + 1] = { kind = "user", text = text, sourceIndex = msg_index } end
    if msg.content and type(msg.content) ~= "string" then
      for i = 1, #msg.content do
        local part = msg.content[i]
        if type(part) == "table" and part.type == "image" then
          blocks[#blocks + 1] = { kind = "user", text = "[image: " .. tostring(part.mimeType) .. "]", sourceIndex = msg_index }
        end
      end
    end
    if #blocks == 0 then blocks[1] = { kind = "user", text = "", sourceIndex = msg_index } end
    return blocks
  end
  if msg.role == "toolResult" then
    return { { kind = "tool_result", name = msg.toolName, text = sanitize(text_of(msg.content)), isError = msg.isError == true, sourceIndex = msg_index } }
  end
  if msg.role == "assistant" then
    if not msg.content then return {} end
    if type(msg.content) == "string" then
      return { { kind = "assistant", text = sanitize(msg.content), sourceIndex = msg_index } }
    end
    local blocks = {}
    for i = 1, #msg.content do
      local part = msg.content[i]
      if type(part) ~= "table" then
      elseif part.type == "text" then
        blocks[#blocks + 1] = { kind = "assistant", text = sanitize(part.text), sourceIndex = msg_index }
      elseif part.type == "thinking" then
        blocks[#blocks + 1] = { kind = "thinking", text = sanitize(part.thinking), redacted = part.redacted == true, sourceIndex = msg_index }
      elseif part.type == "toolCall" then
        blocks[#blocks + 1] = { kind = "tool_call", name = part.name, args = part.arguments, sourceIndex = msg_index }
      end
    end
    return blocks
  end
  return {}
end

local function normalize(messages)
  local out = {}
  for i, msg in ipairs(messages) do
    local blocks = normalize_one(msg, i - 1)
    for _, b in ipairs(blocks) do out[#out + 1] = b end
  end
  return out
end

-- ── Extract: goals ─────────────────────────────────────────────────
local function extract_goals(blocks)
  local goals = {}
  local latest_scope_change = nil
  for _, b in ipairs(blocks) do
    if b.kind ~= "user" then end
    if b.kind == "user" then
      local raw_lines = non_empty_lines(b.text)
      local lines = collapse_skill_lines(raw_lines)
      local kept = {}
      for _, l in ipairs(lines) do
        local t = l:gsub("^%s+", ""):gsub("%s+$", "")
        if t:match("[%[│├└─╭╰]") or t:find("```", 1, true) or t:match("^%s*([=%a]+%(|function |const |let |var |import |export |class )")
           or t:match("^(https?:|file:|/[A-Za-z])") then
        elseif #t > 5 and #t <= 200 and not t:match("^%s*[.!?%s]*$") then
          local stripped = t:gsub("^%s*[-*+%d%.]+%s+", ""):gsub("^%s+", ""):gsub("%s+$", "")
          if #stripped > 5 then kept[#kept + 1] = stripped end
        end
      end
      if #kept == 0 then end
      if #kept > 0 then
        if #goals == 0 then
          for i = 1, math.min(6, #kept) do goals[#goals + 1] = kept[i] end
        else
          local leading = b.text:sub(1, 200)
          if leading:match("%b(instead|actually|change of plan|forget that|new task|switch to|now I want|pivot|let'?s do)") then
            local sc = {}
            for i = 1, math.min(3, #kept) do sc[#sc + 1] = clip(kept[i], 200) end
            latest_scope_change = sc
          elseif leading:match("%b(fix|implement|add|create|build|refactor|debug|investigate|update|remove|delete|migrate|deploy|test|write|set up)") and #kept[1] > 15 then
            local sc = {}
            for i = 1, math.min(2, #kept) do sc[#sc + 1] = clip(kept[i], 200) end
            latest_scope_change = sc
          end
        end
      end
    end
  end
  if latest_scope_change and #latest_scope_change > 0 then
    goals[#goals + 1] = "[Scope change]"
    for _, g in ipairs(latest_scope_change) do goals[#goals + 1] = g end
  end
  local out = {}
  for i = 1, math.min(8, #goals) do out[i] = goals[i] end
  return out
end

-- ── Extract: files ─────────────────────────────────────────────────
local function longest_common_dir_prefix(paths)
  local abs = {}
  for _, p in ipairs(paths) do
    p = tostring(p or "")
    if p:sub(1, 1) == "/" then abs[#abs + 1] = p end
  end
  if #abs < 2 then return "" end
  local split = {}
  for _, p in ipairs(abs) do
    local segs = {}
    for s in p:gmatch("[^/]+") do segs[#segs + 1] = s end
    split[#split + 1] = segs
  end
  local min_len = math.huge
  for _, s in ipairs(split) do min_len = math.min(min_len, #s) end
  local i = 0
  while i < min_len - 1 do
    local seg = split[1][i + 1]
    local all = true
    for _, s in ipairs(split) do if s[i + 1] ~= seg then all = false; break end end
    if not all then break end
    i = i + 1
  end
  if i < 2 then return "" end
  local parts = {}
  for j = 1, i do parts[#parts + 1] = split[1][j] end
  return table.concat(parts, "/") .. "/"
end

local function extract_files(blocks, file_ops)
  local act = { read = {}, modified = {}, created = {} }
  if file_ops then
    for _, p in ipairs(file_ops.readFiles or {}) do act.read[p] = true end
    for _, p in ipairs(file_ops.modifiedFiles or {}) do act.modified[p] = true end
    for _, p in ipairs(file_ops.createdFiles or {}) do act.created[p] = true end
  end
  for _, b in ipairs(blocks) do
    if b.kind == "tool_call" then
      local p = extract_path(b.args)
      if p then
        if ({ Read = true, read_file = true, View = true })[b.name] then act.read[p] = true end
        if ({ Edit = true, Write = true, edit = true, write = true, edit_file = true, write_file = true, MultiEdit = true })[b.name] then act.modified[p] = true end
        if ({ Write = true, write = true, write_file = true })[b.name] then act.created[p] = true end
      end
    end
  end
  local all = {}
  for p in pairs(act.read) do all[#all + 1] = p end
  for p in pairs(act.modified) do all[#all + 1] = p end
  for p in pairs(act.created) do all[#all + 1] = p end
  local prefix = longest_common_dir_prefix(all)
  if prefix ~= "" then
    for _, key in ipairs({ "read", "modified", "created" }) do
      local trimmed = {}
      for p in pairs(act[key]) do
        trimmed[p:sub(1, #prefix) == prefix and p:sub(#prefix + 1) or p] = true
      end
      act[key] = trimmed
    end
  end
  return act
end

-- ── Extract: preferences ───────────────────────────────────────────
local PREF_PATTERNS = {
  "%bprefer",
  "don'?t want",
  "always (use|do|run|prefer|keep|make|format|write|add|set|put|prefix|start|include|append)",
  "never (use|do|run|push|commit|write|ignore|add|set|put|remove|delete|include|deploy)",
  "please (use|avoid|keep|make|don'?t|do not|format|write)",
  "(style|format|language|naming)%s*[:=]%s*%S",
}

local function extract_preferences(blocks)
  local prefs, seen = {}, {}
  for _, b in ipairs(blocks) do
    if b.kind == "user" then
      local per_block = 0
      for _, line in ipairs(non_empty_lines(b.text)) do
        local trimmed = line:gsub("^%s+", ""):gsub("%s+$", "")
        if #trimmed >= 5 and #trimmed <= 200 and not trimmed:match("%?%s*$") and not trimmed:find("?...", 1, true) then
          local matched = false
          for _, p in ipairs(PREF_PATTERNS) do
            if trimmed:match(p) then matched = true break end
          end
          if matched then
            local clipped = clip(trimmed, 200)
            local key = clipped:lower()
            if not seen[key] then
              seen[key] = true
              prefs[#prefs + 1] = clipped
              per_block = per_block + 1
              if per_block >= 1 then break end
            end
          end
        end
      end
    end
  end
  local out = {}
  for i = 1, math.min(10, #prefs) do out[i] = prefs[i] end
  return out
end

local function dedup_preferences_against_goals(prefs, goals)
  local goal_set = {}
  for _, g in ipairs(goals) do goal_set[g:gsub("^%s+", ""):gsub("%s+$", ""):lower()] = true end
  local out = {}
  for _, p in ipairs(prefs) do
    if not goal_set[p:gsub("^%s+", ""):gsub("%s+$", ""):lower()] then out[#out + 1] = p end
  end
  return out
end

-- ── Extract: commits ───────────────────────────────────────────────
local function extract_commits(blocks)
  local commits = {}
  for i = 1, #blocks do
    local b = blocks[i]
    if b.kind == "tool_call" and b.name == "bash" and type(b.args) == "table" and b.args.command then
      local cmd = tostring(b.args.command)
      if cmd:match("%bgit%s+commit") then
        local msg = cmd:match('%-m%s+"((?:[^"\\]|\\.)*)"') or cmd:match("%-m%s+'((?:[^'\\]|\\.)*)'")
        if msg then
          local message = msg:gsub("^[^\n]*", ""):gsub("\\\"", '"'):gsub("\\'", "'"):gsub("^%s+", ""):gsub("%s+$", "")
          local first = message:match("^([^\n\\]+)") or message
          first = first:gsub("^%s+", ""):gsub("%s+$", "")
          if first ~= "" then
            local hash
            for j = i + 1, math.min(#blocks, i + 2) do
              local r = blocks[j]
              if r.kind == "tool_result" and type(r.text) == "string" then
                local bracket = r.text:match("%[%S+%s+([0-9a-f]%f[0-9a-f]%f[0-9a-f]%f[0-9a-f]%f[0-9a-f]%f[0-9a-f]%f[0-9a-f])%]")
                if not bracket then
                  local range = r.text:match("([0-9a-f]%f[%x]-)%.%.([0-9a-f]+)")
                  bracket = range and select(2, r.text:match("([0-9a-f]+)%.%.([0-9a-f]+)")) or nil
                end
                if not bracket then bracket = r.text:match("([0-9a-f]%f[%x]-)") end
                if bracket and #bracket >= 7 and #bracket <= 12 then hash = bracket end
                if hash then break end
              end
            end
            local key = tostring(hash or "") .. "::" .. first
            local dup = false
            for _, c in ipairs(commits) do
              if (tostring(c.hash or "") .. "::" .. c.message) == key then dup = true break end
            end
            if not dup then commits[#commits + 1] = { hash = hash, message = first } end
          end
        end
      end
    end
  end
  return commits
end

local function format_commits(commits, limit)
  limit = limit or 8
  local lines = {}
  local n = #commits
  for idx = math.max(1, n - limit + 1), n do
    local c = commits[idx]
    local prefix = c.hash and (c.hash .. ": ") or ""
    lines[#lines + 1] = prefix .. c.message
  end
  return lines
end

-- ── Format / summary ───────────────────────────────────────────────
local function section(title, items)
  if not items or #items == 0 then return "" end
  local body = {}
  for _, i in ipairs(items) do body[#body + 1] = "- " .. i end
  return "[" .. title .. "]\n" .. table.concat(body, "\n")
end
local BRIEF_MAX_LINES = 120
local RECALL_NOTE = "Use `vcc_recall` to search for prior work, decisions, and context from before this summary. Do not redo work already completed."
local SEPARATOR = "\n\n---\n\n"

local function cap_brief(text)
  local lines = {}
  for l in (text or ""):gmatch("[^\n]+") do lines[#lines + 1] = l end
  if #lines <= BRIEF_MAX_LINES then return text end
  local omitted = #lines - BRIEF_MAX_LINES
  local kept = {}
  for i = #lines - BRIEF_MAX_LINES + 1, #lines do kept[#kept + 1] = lines[i] end
  local first_header = nil
  for i, l in ipairs(kept) do if l:match("^%[.+%]") then first_header = i break end end
  local clean = (first_header and first_header > 1) and (function() local r = {} for i = first_header, #kept do r[#r + 1] = kept[i] end return r end)() or kept
  return "...(" .. omitted .. " earlier lines omitted)\n\n" .. table.concat(clean, "\n")
end

-- ── Build sections ─────────────────────────────────────────────────
local BLOCKER_RE = "%b(fail|broken|cannot|can't|won't work|does not work|doesn't work|still broken|blocked|blocker|crash)"

local function extract_outstanding_context(blocks)
  local items = {}
  local tail = {}
  for i = math.max(1, #blocks - 19), #blocks do tail[#tail + 1] = blocks[i] end
  for _, b in ipairs(tail) do
    if b.kind == "tool_result" and b.isError then
      local item = "[" .. tostring(b.name) .. "] " .. first_line(b.text, 150)
      if not items[item] then items[#items + 1] = item end
    elseif b.kind == "assistant" or b.kind == "user" then
      for _, line in ipairs(non_empty_lines(b.text)) do
        if line:match(BLOCKER_RE) and #line >= 15 then
          if not line:match("^%s*[-*+>]%s") and not line:match("^%s*%(") and line:match("^%s*[\"'`*_]?[A-Z`]") then
            local clipped = (b.kind == "user") and ("[user] " .. clip_sentence(line, 150)) or clip_sentence(line, 150)
            local found = false
            for _, it in ipairs(items) do if it == clipped then found = true break end end
            if not found then items[#items + 1] = clipped end
            break
          end
        end
      end
    end
  end
  local out = {}
  for i = 1, math.min(5, #items) do out[i] = items[i] end
  return out
end

local function format_file_activity(blocks)
  local act = extract_files(blocks)
  for p in pairs(act.modified) do act.created[p] = nil end
  local lines = {}
  local function cap(set, limit)
    local arr = {}
    for p in pairs(set) do arr[#arr + 1] = p end
    table.sort(arr)
    if #arr <= limit then return table.concat(arr, ", ") end
    local head = {}
    for i = 1, limit do head[i] = arr[i] end
    return table.concat(head, ", ") .. " (+" .. (#arr - limit) .. " more)"
  end
  local has = false
  for _ in pairs(act.modified) do has = true break end
  if has then lines[#lines + 1] = "Modified: " .. cap(act.modified, 10) end
  has = false
  for _ in pairs(act.created) do has = true break end
  if has then lines[#lines + 1] = "Created: " .. cap(act.created, 10) end
  has = false
  for _ in pairs(act.read) do has = true break end
  if has then lines[#lines + 1] = "Read: " .. cap(act.read, 10) end
  return lines
end

-- ── Brief (transcript) ─────────────────────────────────────────────
local function truncate_brief_tokens(text, limit)
  local flat = tostring(text or ""):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
  local stop = {}
  for _, w in ipairs({ "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can", "need", "must", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through", "during", "before", "after", "above", "below", "between", "under", "over", "and", "but", "or", "nor", "not", "so", "yet", "both", "either", "neither", "each", "every", "all", "any", "few", "more", "most", "other", "some", "such", "no", "that", "this", "these", "those", "it", "its", "i", "me", "my", "we", "our", "you", "your", "he", "him", "his", "she", "her", "they", "them", "their", "who", "which", "what", "if", "then", "than", "when", "where", "how", "just", "also" }) do stop[w] = true end
  local count = 0
  local last_end = 0
  local pos = 1
  while true do
    local word_start, word_end, word = flat:find("(%S+)%s*", pos)
    if not word then break end
    if not stop[word:lower()] then
      count = count + 1
      if count > limit then
        return flat:sub(1, last_end):gsub("%s+$", "") .. "...(truncated)"
      end
    end
    last_end = word_end
    pos = word_end + 1
  end
  return flat
end

local BASH_CAP = 120
local function compress_bash(raw)
  local first
  for line in tostring(raw or ""):gmatch("[^\n]+") do
    local t = line:gsub("^%s+", ""):gsub("%s+$", "")
    if t ~= "" then first = t break end
  end
  local cmd = first or tostring(raw or "")
  cmd = cmd:gsub("^cd%s+%S+%s*&&%s*", "")
  for _ = 1, 3 do
    local stripped = cmd:gsub("%s*|%s*(head|tail|sort|wc|column|tr|cut|awk|uniq|python3|node|bun)[^\n]*$", "")
    if stripped == cmd then break end
    cmd = stripped
  end
  if #cmd > BASH_CAP then return cmd:sub(1, BASH_CAP - 3) .. "..." end
  return cmd
end

local TOOL_SUMMARY_FIELDS = { Read = "file_path", Edit = "file_path", Write = "file_path", read = "file_path", edit = "file_path", write = "file_path", Glob = "pattern", Grep = "pattern" }

local function tool_one_liner(name, args)
  local field = TOOL_SUMMARY_FIELDS[name]
  if field and type(args) == "table" and type(args[field]) == "string" then
    return "* " .. name .. ' "' .. args[field] .. '"'
  end
  local path = extract_path(args)
  if path then return "* " .. name .. ' "' .. path .. '"' end
  if name == "bash" or name == "Bash" then
    local raw = type(args) == "table" and (args.command or args.description) or ""
    return "* " .. name .. ' "' .. compress_bash(raw) .. '"'
  end
  if type(args) == "table" and type(args.query) == "string" then
    return "* " .. name .. ' "' .. clip(args.query, 60) .. '"'
  end
  return "* " .. name
end

local SELF_TALK_PREFIX_RE = "^%s*(hmm|wait|actually|oh|okay|ok|well|so)[,.!%-]"

local function build_brief_sections(blocks)
  local sections = {}
  local last_header = ""
  local function push(header, line)
    if header == last_header and #sections > 0 then
      sections[#sections].lines[#sections[#sections].lines + 1] = line
    else
      sections[#sections + 1] = { header = header, lines = { line } }
      last_header = header
    end
  end
  for _, b in ipairs(blocks) do
    if b.kind == "user" then
      local trimmed = tostring(b.text or ""):gsub("^%s+", ""):gsub("%s+$", "")
      if trimmed ~= "" then
        local text = truncate_brief_tokens(collapse_skill_text(b.text), 256)
        if text ~= "" then
          local ref = (b.sourceIndex ~= nil) and (" (#" .. tostring(b.sourceIndex) .. ")") or ""
          push("[user]", text .. ref)
        end
      end
      last_header = "[user]"
    elseif b.kind == "assistant" then
      local raw = tostring(b.text or "")
      for _ = 1, 2 do
        local stripped = raw:gsub(SELF_TALK_PREFIX_RE, "")
        if stripped == raw then break end
        raw = stripped
      end
      local text = truncate_brief_tokens(raw, 200)
      if text ~= "" then
        local ref = (b.sourceIndex ~= nil) and (" (#" .. tostring(b.sourceIndex) .. ")") or ""
        push("[assistant]", text .. ref)
      end
    elseif b.kind == "tool_call" then
      if b.name and b.name:gsub("%s", "") ~= "" then
        local ref = (b.sourceIndex ~= nil) and (" (#" .. tostring(b.sourceIndex) .. ")") or ""
        push("[assistant]", tool_one_liner(b.name, b.args) .. ref)
      end
    elseif b.kind == "tool_result" then
      if b.isError then
        local body = first_line(b.text, 150)
        if body ~= "" and body ~= "(no output)" then
          local ref = (b.sourceIndex ~= nil) and (" (#" .. tostring(b.sourceIndex) .. ")") or ""
          local header = "[tool_error] " .. tostring(b.name) .. ref
          push(header, body)
          last_header = header
        end
      end
    end
  end
  -- cap tool calls per [assistant] turn
  for _, sec in ipairs(sections) do
    if sec.header ~= "[assistant]" then end
    if sec.header == "[assistant]" then
      local tool_idx = {}
      for i, l in ipairs(sec.lines) do if l:sub(1, 2) == "* " then tool_idx[#tool_idx + 1] = i end end
      if #tool_idx > 8 then
        local drop_count = #tool_idx - 8
        local drop_set = {}
        for i = 1, drop_count do drop_set[tool_idx[i]] = true end
        local first_kept = tool_idx[drop_count + 1]
        local next = {}
        local inserted = false
        for i, l in ipairs(sec.lines) do
          if drop_set[i] then
          elseif inserted == false and i == first_kept then
            next[#next + 1] = "* (" .. drop_count .. " earlier tool-call entries omitted)"
            inserted = true
            next[#next + 1] = l
          else
            next[#next + 1] = l
          end
        end
        sec.lines = next
      end
    end
  end
  return sections
end

local function stringify_brief(sections)
  local out = {}
  for i, sec in ipairs(sections) do
    if i > 1 then
      local prev = sections[i - 1]
      local prev_tools = prev.header == "[assistant]" and #prev.lines > 0
      for _, l in ipairs(prev.lines) do if l:sub(1, 2) ~= "* " then prev_tools = false break end end
      local cur_tools = sec.header == "[assistant]"
      if #sec.lines > 0 then for _, l in ipairs(sec.lines) do if l:sub(1, 2) ~= "* " then cur_tools = false break end end end
      if not (prev_tools and cur_tools) then out[#out + 1] = "" end
    end
    out[#out + 1] = sec.header
    for _, line in ipairs(sec.lines) do out[#out + 1] = line end
  end
  return table.concat(out, "\n")
end

local function sections_to_transcript(sections)
  local entries = {}
  for _, sec in ipairs(sections) do
    if sec.header == "[user]" then
      for _, line in ipairs(sec.lines) do
        local ref = line:match("%s*%((#%d+)%)$")
        local clean = ref and line:sub(1, -(#ref + 3)):gsub("%s+$", "") or line
        local e = { role = "user", text = clean }
        if ref then e.ref = ref end
        entries[#entries + 1] = e
      end
    elseif sec.header == "[assistant]" then
      for _, line in ipairs(sec.lines) do
        if line:sub(1, 2) == "* " then
          local tool = line:match("^%* (%S+)")
          local cmd = line:match('^%* %S+ "([^"]*)"')
          local ref = line:match("%((#%d+(?:, ?#%d+)*)%)%s*(x%d+)?$")
          local count = line:match("x(%d+)$")
          if tool then
            local e = { role = "assistant", tool = tool }
            if cmd then e.cmd = cmd end
            if ref then e.ref = ref elseif line:match("%(#%d+%)$") then e.ref = line:match("%(#(%d+)%)$") and "#" .. line:match("%(#(%d+)%)$") end
            if count then e.count = tonumber(count) end
            entries[#entries + 1] = e
          end
        else
          local ref = line:match("%s*%((#%d+)%)$")
          local clean = ref and line:sub(1, -(#ref + 3)):gsub("%s+$", "") or line
          local e = { role = "assistant", text = clean }
          if ref then e.ref = ref end
          entries[#entries + 1] = e
        end
      end
    elseif sec.header:gsub("%s", ""):sub(1, 12) == "[tool_error]" then
      local tool = sec.header:match("^%[tool_error%]%s+(%S+)")
      for _, line in ipairs(sec.lines) do
        entries[#entries + 1] = { role = "tool_error", tool = tool or "unknown", text = line }
      end
    end
  end
  return entries
end

local function build_brief_sections_and_string(blocks)
  local sections = build_brief_sections(blocks)
  return stringify_brief(sections), sections
end

-- ── Build sections (full) ──────────────────────────────────────────
local function build_sections(blocks)
  local brief_text, sections_entries = build_brief_sections_and_string(blocks)
  local session_goal = extract_goals(blocks)
  local user_preferences = dedup_preferences_against_goals(extract_preferences(blocks), session_goal)
  return {
    sessionGoal = session_goal,
    outstandingContext = extract_outstanding_context(blocks),
    filesAndChanges = format_file_activity(blocks),
    commits = format_commits(extract_commits(blocks)),
    userPreferences = user_preferences,
    briefTranscript = brief_text,
    transcriptEntries = sections_to_transcript(sections_entries),
  }
end

-- ── Format summary ─────────────────────────────────────────────────
local function format_summary(data)
  local header_parts = {}
  local function add(title, items) if items and #items > 0 then header_parts[#header_parts + 1] = section(title, items) end end
  add("Session Goal", data.sessionGoal)
  add("Files And Changes", data.filesAndChanges)
  add("Commits", data.commits)
  add("Outstanding Context", data.outstandingContext)
  add("User Preferences", data.userPreferences)
  local parts = {}
  if #header_parts > 0 then parts[#parts + 1] = table.concat(header_parts, "\n\n") end
  if data.briefTranscript and data.briefTranscript ~= "" then parts[#parts + 1] = cap_brief(data.briefTranscript) end
  return table.concat(parts, "\n\n---\n\n")
end

-- ── Summarize (compile + merge) ────────────────────────────────────
local HEADER_NAMES = { "Session Goal", "Files And Changes", "Commits", "Outstanding Context", "User Preferences" }

local function strip_recall_note(text)
  local idx = text:find(RECALL_NOTE, 1, true)
  if not idx then return text end
  local head = text:sub(1, idx - 1)
  head = head:gsub("%s*%(%s*\n\n---\n\n%s*)?$", "")
  head = head:gsub("%s+$", "")
  return head
end

local function section_of(text, header)
  local tag = "[" .. header .. "]"
  local start = text:find(tag, 1, true)
  if not start then return "" end
  local after = text:sub(start)
  local candidates = {}
  for _, h in ipairs(HEADER_NAMES) do
    if h ~= header then
      local pos = after:find("[" .. h .. "]", 1, true)
      if pos and pos > 0 then candidates[#candidates + 1] = pos end
    end
  end
  local sep = after:find("\n\n---\n\n", 1, true)
  if sep then candidates[#candidates + 1] = sep end
  table.sort(candidates)
  local endp = candidates[1]
  if endp then return after:sub(1, endp - 1):gsub("^%s+", ""):gsub("%s+$", "") end
  return after:gsub("^%s+", ""):gsub("%s+$", "")
end

local function brief_of(text)
  local idx = text:find(SEPARATOR, 1, true)
  if not idx then return "" end
  return text:sub(idx + #SEPARATOR):gsub("^%s+", ""):gsub("%s+$", "")
end

local function merge_file_lines(prev, fresh)
  local merged = { Modified = {}, Created = {}, Read = {} }
  local function parse(text)
    for line in (text or ""):gmatch("[^\n]+") do
      for _, cat in ipairs({ "Modified", "Created", "Read" }) do
        local prefix = "- " .. cat .. ": "
        if line:sub(1, #prefix) == prefix then
          local rest = line:sub(#prefix + 1)
          rest = rest:gsub("%s*(%(%+%d+ more%))%s*$", "")
          for p in rest:gmatch("[^,]+") do
            local trimmed = p:gsub("^%s+", ""):gsub("%s+$", "")
            if trimmed ~= "" then merged[cat][trimmed] = true end
          end
        end
      end
    end
  end
  parse(prev)
  parse(fresh)
  for p in pairs(merged.Modified) do merged.Created[p] = nil end
  local function cap(set, limit)
    local arr = {}
    for p in pairs(set) do arr[#arr + 1] = p end
    table.sort(arr)
    if #arr <= limit then return table.concat(arr, ", ") end
    local head = {}
    for i = 1, limit do head[i] = arr[i] end
    return table.concat(head, ", ") .. " (+" .. (#arr - limit) .. " more)"
  end
  local lines = {}
  local has = false
  for _ in pairs(merged.Modified) do has = true break end
  if has then lines[#lines + 1] = "- Modified: " .. cap(merged.Modified, 10) end
  has = false
  for _ in pairs(merged.Created) do has = true break end
  if has then lines[#lines + 1] = "- Created: " .. cap(merged.Created, 10) end
  has = false
  for _ in pairs(merged.Read) do has = true break end
  if has then lines[#lines + 1] = "- Read: " .. cap(merged.Read, 10) end
  if #lines == 0 then return "" end
  return "[Files And Changes]\n" .. table.concat(lines, "\n")
end

local function merge_header_section(header, prev, fresh)
  if header == "Outstanding Context" then return fresh end
  if not prev or prev == "" then return fresh end
  if not fresh or fresh == "" then return prev end
  if header == "Files And Changes" then return merge_file_lines(prev, fresh) end
  local function is_clean(l)
    return l:sub(1, 2) == "- " and not l:find("<skill", 1, true) and not l:find("</skill", 1, true)
  end
  local combined = {}
  local seen = {}
  local function add_lines(text)
    for line in (text or ""):gmatch("[^\n]+") do
      if is_clean(line) and not seen[line] then seen[line] = true; combined[#combined + 1] = line end
    end
  end
  add_lines(prev)
  add_lines(fresh)
  local CAP = (header == "Session Goal" or header == "Commits") and 8 or 15
  local capped = {}
  local start = math.max(1, #combined - CAP + 1)
  for i = start, #combined do capped[#capped + 1] = combined[i] end
  if #capped == 0 then return "" end
  return "[" .. header .. "]\n" .. table.concat(capped, "\n")
end

local function merge_brief_transcript(prev, fresh)
  if not prev or prev == "" then return fresh end
  if not fresh or fresh == "" then return prev end
  return prev .. "\n\n" .. fresh
end

local function merge_previous(prev, fresh)
  local headers = {}
  for _, header in ipairs(HEADER_NAMES) do
    local merged = merge_header_section(header, section_of(prev, header), section_of(fresh, header))
    if merged ~= "" then headers[#headers + 1] = merged end
  end
  local merged_brief = merge_brief_transcript(brief_of(prev), brief_of(fresh))
  local parts = {}
  if #headers > 0 then parts[#parts + 1] = table.concat(headers, "\n\n") end
  if merged_brief and merged_brief ~= "" then parts[#parts + 1] = cap_brief(merged_brief) end
  return table.concat(parts, SEPARATOR)
end

local function compile(input)
  local messages = input.messages or {}
  local blocks = filter_noise(normalize(messages))
  local data = build_sections(blocks)
  local fresh = format_summary(data)
  local prev = input.previousSummary and strip_recall_note(input.previousSummary) or nil
  local merged = (prev and prev ~= "") and merge_previous(prev, fresh) or fresh
  if (not merged) or merged == "" then return "" end
  return merged .. SEPARATOR .. RECALL_NOTE
end

-- ── Search entries (regex + BM25-lite) ─────────────────────────────
local function is_regex(str) return tostring(str or ""):match("[|*+?{}()[%]\\^$.]") ~= nil end

local function build_bm25_context(docs, terms)
  local n = #docs
  local df = {}
  local total_len = 0
  for _, doc in ipairs(docs) do
    local count = 0
    for _ in tostring(doc or ""):gmatch("%S+") do count = count + 1 end
    total_len = total_len + count
    for _, t in ipairs(terms) do
      if tostring(doc or ""):find(t, 1, true) then df[t] = (df[t] or 0) + 1 end
    end
  end
  return { n = n, avgDl = total_len / math.max(n, 1), df = df }
end

local function bm25_score(doc, terms, ctx)
  local dl = 0
  for _ in tostring(doc or ""):gmatch("%S+") do dl = dl + 1 end
  local score = 0
  for _, t in ipairs(terms) do
    local tf = 0
    local pos = 1
    while true do
      local s = tostring(doc or ""):find(t, pos, true)
      if not s then break end
      tf = tf + 1
      pos = s + #t
    end
    if tf > 0 then
      local doc_freq = ctx.df[t] or 0
      local idf = math.log((ctx.n - doc_freq + 0.5) / (doc_freq + 0.5) + 1)
      local tf_norm = (tf * 2.2) / (tf + 1.2 * (1 - 0.75 + 0.75 * dl / ctx.avgDl))
      score = score + idf * tf_norm
    end
  end
  return score
end

local function count_matches(hay, terms)
  local count = 0
  for _, t in ipairs(terms) do
    if tostring(hay or ""):find(t, 1, true) then count = count + 1 end
  end
  return count
end

local function line_snippet(text, term)
  local lines = {}
  for l in (text or ""):gmatch("[^\n]+") do lines[#lines + 1] = l end
  local match_idx = nil
  for i, l in ipairs(lines) do
    if l:find(term, 1, true) or term:match("^%[") and l:find(term, 1, true) then match_idx = i break end
  end
  if not match_idx then match_idx = nil end
  if not match_idx then return nil end
  local start = math.max(1, match_idx - 2)
  local endp = math.min(#lines, match_idx + 2)
  local slice = {}
  for i = start, endp do slice[#slice + 1] = lines[i] end
  local parts = {}
  if start > 1 then parts[#parts + 1] = "...(" .. (start - 1) .. " lines above)" end
  for _, s in ipairs(slice) do parts[#parts + 1] = s end
  if endp < #lines then parts[#parts + 1] = "...(" .. (#lines - endp) .. " lines below)" end
  return table.concat(parts, "\n")
end

local STOPWORDS = {}
for _, w in ipairs({ "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does", "did", "will", "would", "could", "should", "may", "might", "can", "shall", "of", "in", "to", "for", "with", "on", "at", "from", "by", "as", "into", "through", "during", "before", "after", "above", "below", "between", "out", "off", "over", "under", "again", "further", "then", "once", "here", "there", "when", "where", "why", "how", "all", "both", "each", "few", "more", "most", "other", "some", "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too", "very", "just", "about", "it", "its", "that", "this", "what", "which", "who", "whom", "these", "those" }) do STOPWORDS[w] = true end

local function filter_stopwords(terms)
  local meaningful = {}
  for _, t in ipairs(terms) do
    if not STOPWORDS[t:lower()] and #t > 1 then meaningful[#meaningful + 1] = t end
  end
  return #meaningful > 0 and meaningful or terms
end

local function full_text(msg)
  if type(msg) == "table" and msg.role == "bashExecution" then
    return tostring(msg.command or "") .. " " .. tostring(msg.output or "")
  end
  return text_of(type(msg) == "table" and msg.content or msg)
end

local function search_entries(entries, messages, query)
  query = query or ""
  if query:gsub("%s", "") == "" then return entries end
  local raw_query = query:gsub("^%s+", ""):gsub("%s+$", "")
  if is_regex(raw_query) then
    local hits = {}
    for i = 1, #entries do
      local e = entries[i]
      local msg = messages[i]
      local text = msg and full_text(msg) or e.summary
      local file_part = e.files and table.concat(e.files, " ") or ""
      local hay = e.role .. " " .. text .. " " .. file_part
      if hay:find(raw_query) or hay:lower():find(raw_query:lower()) then
        local snip = line_snippet(text, raw_query)
        hits[#hits + 1] = { index = e.index, role = e.role, summary = e.summary, files = e.files, snippet = snip, matchCount = 1 }
      end
    end
    return hits
  end
  local raw_terms = {}
  for t in raw_query:gmatch("%S+") do raw_terms[#raw_terms + 1] = t end
  local terms = filter_stopwords(raw_terms)
  local docs = {}
  for i = 1, #entries do
    local e = entries[i]
    local msg = messages[i]
    local text = msg and full_text(msg) or e.summary
    local file_part = e.files and table.concat(e.files, " ") or ""
    docs[i] = e.role .. " " .. text .. " " .. file_part
  end
  local ctx = build_bm25_context(docs, terms)
  local scored = {}
  for i = 1, #entries do
    local e = entries[i]
    local hay = docs[i]
    local mc = count_matches(hay, terms)
    if mc > 0 then
      local score = bm25_score(hay, terms, ctx)
      local msg = messages[i]
      local text = msg and full_text(msg) or e.summary
      local snip = line_snippet(text, terms[1])
      scored[#scored + 1] = { hit = { index = e.index, role = e.role, summary = e.summary, files = e.files, snippet = snip, matchCount = mc }, score = score }
    end
  end
  table.sort(scored, function(a, b) return b.score > a.score end)
  local out = {}
  for _, s in ipairs(scored) do out[#out + 1] = s.hit end
  return out
end

local function format_recall_output(entries, query, header_override)
  if #entries == 0 then
    if query and query:gsub("%s", "") ~= "" then return 'No matches for "' .. query .. '" in session history.' end
    return "No entries in session history."
  end
  local header
  if header_override then
    header = header_override .. (query and (' for "' .. query .. '":') or '')
  elseif query and query:gsub("%s", "") ~= "" then
    header = 'Found ' .. #entries .. ' matches for "' .. query .. '":'
  else
    header = "Session history (" .. #entries .. " entries):"
  end
  local lines = {}
  for _, e in ipairs(entries) do
    local file_suffix = e.files and #e.files > 0 and (" files:[" .. table.concat(e.files, ", ") .. "]") or ""
    local body = (query and e.snippet) and e.snippet or e.summary
    lines[#lines + 1] = "#" .. tostring(e.index) .. " [" .. e.role .. "]" .. file_suffix .. " " .. tostring(body)
  end
  return header .. "\n\n" .. table.concat(lines, "\n\n")
end

-- ── Report ─────────────────────────────────────────────────────────
local function build_compact_report(input)
  local summary = compile(input)
  local data = build_sections(normalize(input.messages or {}))
  local input_chars = 0
  local rendered_all = {}
  for i, msg in ipairs(input.messages or {}) do
    local r = render_message(msg, i - 1, true)
    rendered_all[i] = r
    input_chars = input_chars + #r.summary
  end
  local function est_tokens(c) return math.ceil(c / 4) end
  local role_counts = { user = 0, assistant = 0, toolResult = 0 }
  for _, msg in ipairs(input.messages or {}) do
    if msg.role == "user" then role_counts.user = role_counts.user + 1
    elseif msg.role == "assistant" then role_counts.assistant = role_counts.assistant + 1
    elseif msg.role == "toolResult" then role_counts.toolResult = role_counts.toolResult + 1 end
  end
  local top_files = {}
  for _, b in ipairs(normalize(input.messages or {})) do
    if b.kind == "tool_call" then
      local p = extract_path(b.args)
      if p and #top_files < 10 then top_files[#top_files + 1] = p end
    end
  end
  local brief_line_count = 0
  local sep_idx = summary:find(SEPARATOR, 1, true)
  if sep_idx then
    for _ in summary:sub(sep_idx + #SEPARATOR):gmatch("[^\n]+") do brief_line_count = brief_line_count + 1 end
  end
  return {
    summary = summary,
    before = {
      messageCount = #(input.messages or {}),
      roleCounts = role_counts,
      blockCounts = { user = 0, assistant = 0, toolCalls = 0, toolResults = 0, thinking = 0 },
      inputChars = input_chars,
      estimatedTokens = est_tokens(input_chars),
      topFiles = top_files,
      preview = "",
    },
    after = {
      summaryLength = #summary,
      estimatedTokens = est_tokens(#summary),
      sectionCount = 0,
      summaryPreview = summary,
      goalsCount = #data.sessionGoal,
      blockersCount = #data.outstandingContext,
      briefTranscriptLines = brief_line_count,
    },
    compression = {
      charsBefore = input_chars,
      charsAfter = #summary,
      ratio = (#summary == 0) and 0 or tonumber(string.format("%.2f", input_chars / #summary)),
      messagesBefore = #(input.messages or {}),
    },
    recall = { probes = {} },
  }
end

-- ── Before-compact hook ────────────────────────────────────────────
local PI_VCC_COMPACT_INSTRUCTION = "__pi_vcc__"
local last_stats = nil
local last_compact_was_pi_vcc = false
local function get_last_compaction_stats() return last_stats end

local function build_own_cut(branch_entries)
  local last_compaction_idx = -1
  local last_kept_id = nil
  for i = #branch_entries, 1, -1 do
    local e = branch_entries[i]
    if type(e) == "table" and e.type == "compaction" then
      last_compaction_idx = i
      last_kept_id = e.firstKeptEntryId
      break
    end
  end
  local has_prior = last_compaction_idx >= 0
  local has_valid = last_kept_id and (function() for _, e in ipairs(branch_entries) do if e.id == last_kept_id then return true end end return false end)() or false
  local orphan = has_prior and not has_valid

  local live_messages = {}
  if orphan then
    for i = last_compaction_idx + 1, #branch_entries do
      local e = branch_entries[i]
      if e.type == "message" and e.message then live_messages[#live_messages + 1] = { entry = e, message = e.message } end
    end
  else
    local found_kept = (not last_kept_id)
    for _, e in ipairs(branch_entries) do
      if not found_kept and e.id == last_kept_id then found_kept = true end
      if found_kept then
        if e.type == "message" and e.message then live_messages[#live_messages + 1] = { entry = e, message = e.message } end
      end
    end
  end
  if #live_messages == 0 then return { ok = false, reason = "no_live_messages" } end
  if #live_messages <= 2 then return { ok = false, reason = "too_few_live_messages" } end

  local cut_idx = #live_messages
  while cut_idx > 0 and live_messages[cut_idx].message.role ~= "user" do cut_idx = cut_idx - 1 end
  if cut_idx <= 0 then
    for _, m in ipairs(live_messages) do
      if m.message.role == "user" then
        local out = {}
        for _, e in ipairs(live_messages) do out[#out + 1] = e.message end
        return { ok = true, messages = out, firstKeptEntryId = "", compactAll = true }
      end
    end
    return { ok = false, reason = "no_user_message" }
  end
  local msgs = {}
  for i = 1, cut_idx do msgs[#msgs + 1] = live_messages[i].message end
  return { ok = true, messages = msgs, firstKeptEntryId = live_messages[cut_idx].entry.id, compactAll = false }
end

local function preview_content(content)
  if type(content) == "string" then return content:sub(1, 300) end
  if type(content) == "table" then
    local parts = {}
    for i = 1, #content do
      local c = content[i]
      if type(c) == "table" then
        if c.type == "text" then parts[#parts + 1] = c.text or ""
        elseif c.type == "toolCall" then parts[#parts + 1] = "[toolCall:" .. tostring(c.name) .. "]"
        elseif c.type == "thinking" then parts[#parts + 1] = "[thinking]"
        elseif c.type == "image" then parts[#parts + 1] = "[image:" .. tostring(c.mimeType) .. "]"
        else parts[#parts + 1] = "[" .. tostring(c.type or "unknown") .. "]" end
      end
    end
    return table.concat(parts, "\n"):sub(1, 300)
  end
  return ""
end

local function estimate_kept(branch_entries, first_kept_entry_id)
  local kept_idx = nil
  for i, e in ipairs(branch_entries) do if e.id == first_kept_entry_id then kept_idx = i break end end
  if not kept_idx then return 0, 0 end
  local count, chars = 0, 0
  for i = kept_idx, #branch_entries do
    local e = branch_entries[i]
    if e.type == "message" and e.message then
      count = count + 1
      local c = e.message.content
      if type(c) == "string" then chars = chars + #c
      elseif type(c) == "table" then
        for _, p in ipairs(c) do
          if p.text then chars = chars + #p.text
          elseif p.type == "toolCall" then chars = chars + (#(p.name or "") + 0)
          end
        end
      end
    end
  end
  return count, chars
end

local function format_tokens(n)
  if n >= 1000 then return string.format("%.1fk", n / 1000) end
  return tostring(n)
end

local function register_before_compact_hook()
  pi.on("session_before_compact", function(event, ctx)
    local preparation = event.preparation or {}
    local branch_entries = event.branchEntries or {}
    local custom_instructions = event.customInstructions
    local settings = load_settings()
    local is_pi_vcc = custom_instructions == PI_VCC_COMPACT_INSTRUCTION
    if not is_pi_vcc and not settings.overrideDefaultCompaction then return end

    local own_cut = build_own_cut(branch_entries)
    if not own_cut.ok then
      local ok_notify = pcall(function()
        if ctx and ctx.ui and ctx.ui.notify then
          local reason_msgs = {
            no_live_messages = "pi-vcc: Nothing to compact (no live messages)",
            too_few_live_messages = "pi-vcc: Too few messages to compact",
            no_user_message = "pi-vcc: Cannot compact — no user message found",
          }
          ctx.ui.notify(reason_msgs[own_cut.reason] or own_cut.reason, "warning")
        end
      end)
      if not ok_notify then end
      return { cancel = true }
    end

    local agent_messages = own_cut.messages
    local first_kept_entry_id = own_cut.firstKeptEntryId
    local messages = convert_to_llm(agent_messages)
    local kept_count, kept_chars = estimate_kept(branch_entries, first_kept_entry_id)
    last_stats = {
      summarized = #agent_messages,
      kept = kept_count,
      keptTokensEst = math.round(kept_chars / 4),
    }

    local previous_summary = preparation.previousSummary
    local file_ops = preparation.fileOps or {}
    local modified = {}
    for _, p in ipairs(file_ops.written or {}) do modified[#modified + 1] = p end
    for _, p in ipairs(file_ops.edited or {}) do modified[#modified + 1] = p end
    local summary = compile({
      messages = messages,
      previousSummary = previous_summary,
      fileOps = {
        readFiles = file_ops.read or {},
        modifiedFiles = modified,
      },
    })

    local sections = {}
    for m in summary:gmatch("^%[([^%]]+)%]") do sections[#sections + 1] = m end
    local details = {
      compactor = "pi-vcc",
      version = 1,
      sections = sections,
      sourceMessageCount = #agent_messages,
      previousSummaryUsed = (previous_summary ~= nil) and (tostring(previous_summary) ~= "") or false,
    }
    last_compact_was_pi_vcc = is_pi_vcc

    return {
      compaction = {
        summary = summary,
        details = details,
        tokensBefore = preparation.tokensBefore,
        firstKeptEntryId = first_kept_entry_id,
      },
    }
  end)

  pi.on("session_compact", function(event, ctx)
    if not event.fromExtension then return end
    if last_compact_was_pi_vcc then return end
    local stats = last_stats
    if not stats then return end
    pi.set_timeout(function()
      local ok_notify = pcall(function()
        if ctx and ctx.ui and ctx.ui.notify then
          ctx.ui.notify("pi-vcc: " .. stats.summarized .. " source entries processed; tail kept " .. stats.kept .. " (~" .. format_tokens(stats.keptTokensEst) .. " tok).", "info")
        end
      end)
      if not ok_notify then end
    end, 500)
  end)
end

-- ── Commands & tool ────────────────────────────────────────────────
local PAGE_SIZE = 5
local DEFAULT_RECENT = 25

pi.register_command("pi-vcc", {
  description = "Compact conversation with pi-vcc structured summary",
  handler = function(_args, ctx)
    local ok_compact = pcall(function()
      ctx.compact({
        customInstructions = PI_VCC_COMPACT_INSTRUCTION,
        onComplete = function()
          local stats = get_last_compaction_stats()
          if stats then
            ctx.ui.notify("pi-vcc: " .. stats.summarized .. " source entries processed; tail kept " .. stats.kept .. " (~" .. format_tokens(stats.keptTokensEst) .. " tok).", "info")
          else
            ctx.ui.notify("Compacted with pi-vcc", "info")
          end
        end,
        onError = function(err)
          local msg = err and err.message or ""
          if msg == "Compaction cancelled" or msg == "Already compacted" then
            ctx.ui.notify("Nothing to compact", "warning")
          else
            ctx.ui.notify("Compaction failed: " .. msg, "error")
          end
        end,
      })
    end)
    if not ok_compact then
      ctx.ui.notify("Compaction failed", "error")
    end
  end,
})

pi.register_command("pi-vcc-recall", {
  description = "Search session history. Defaults to active lineage; add scope:all for off-lineage branches.",
  handler = function(args, ctx)
    local session_file = ctx.sessionManager.getSessionFile()
    if not session_file then
      ctx.ui.notify("No session file available.", "error")
      return
    end
    local raw = args:gsub("^%s+", ""):gsub("%s+$", "")
    local parsed = parse_recall_scope(raw or "")
    local lineage_entry_ids = (parsed.scope == "lineage") and get_active_lineage_entry_ids(ctx.sessionManager) or nil
    local page_match = parsed.text:match("%bpage:(%d+)")
    local query = parsed.text:gsub("%bpage:%d+", ""):gsub("^%s+", ""):gsub("%s+$", "")
    local function send_recent()
      local loaded = load_all_messages(session_file, false, lineage_entry_ids)
      local recent = {}
      for i = math.max(1, #loaded.rendered - DEFAULT_RECENT + 1), #loaded.rendered do recent[#recent + 1] = loaded.rendered[i] end
      local output = (parsed.scope == "all" and "Scope: all\n\n" or "") .. format_recall_output(recent)
      pi.sendMessage({ customType = "vcc-recall", content = output, display = true }, { triggerTurn = true })
    end
    if not parsed.text or parsed.text == "" then send_recent() return end
    local page = page_match and math.max(1, tonumber(page_match)) or 1
    if not query or query == "" then send_recent() return end
    local loaded = load_all_messages(session_file, false, lineage_entry_ids)
    local all_results = search_entries(loaded.rendered, loaded.rawMessages, query)
    local start = (page - 1) * PAGE_SIZE
    local page_results = {}
    for i = start + 1, math.min(#all_results, start + PAGE_SIZE) do page_results[#page_results + 1] = all_results[i] end
    local total_pages = math.max(1, math.ceil(#all_results / PAGE_SIZE))
    local scope_suffix = (parsed.scope == "all") and " (scope: all)" or ""
    local header = (total_pages > 1) and ("Page " .. page .. "/" .. total_pages .. " (" .. #all_results .. " total matches" .. scope_suffix .. ")") or (#all_results .. " matches" .. scope_suffix)
    local footer = (page < total_pages) and ("\n--- /pi-vcc-recall " .. query .. (parsed.scope == "all" and " scope:all" or "") .. " page:" .. (page + 1) .. " ---") or ""
    local output = format_recall_output(page_results, query, header) .. footer
    pi.sendMessage({ customType = "vcc-recall", content = output, display = true }, { triggerTurn = true })
  end,
})

pi.register_tool({
  name = "vcc_recall",
  label = "VCC Recall",
  description = "Search session history. Defaults to active lineage; use scope:'all' to include off-lineage branches. Supports regex queries, paging, and expand indices.",
  promptSnippet = "vcc_recall: Search history; default scope is active lineage. Use scope:'all' for off-lineage branches.",
  parameters = {
    type = "object",
    properties = {
      query = { type = "string", description = "Search terms or regex pattern (e.g. 'hook|inject', 'fail.*build'). Multi-word = OR ranked by relevance." },
      expand = { type = "array", items = { type = "number" }, description = "Entry indices to return full untruncated content for" },
      page = { type = "number", description = "Page number (1-based) for paginated search results. Default: 1." },
      scope = { type = "string", enum = { "lineage", "all" }, description = "Search scope. Default: lineage; all includes off-lineage branches." },
    },
  },
  execute = function(_tool_call_id, params, _signal, _on_update, ctx)
    local session_file = ctx.sessionManager.getSessionFile()
    if not session_file then
      return { content = { { type = "text", text = "No session file available." } }, details = nil }
    end
    local scope = normalize_recall_scope(params.scope)
    local lineage_entry_ids = (scope == "lineage") and get_active_lineage_entry_ids(ctx.sessionManager) or nil
    local expand_ids = {}
    if type(params.expand) == "table" then for _, n in ipairs(params.expand) do expand_ids[n] = true end end
    local has_expand = next(expand_ids) ~= nil
    if has_expand and not (params.query and params.query:gsub("%s", "") ~= "") then
      local loaded = load_all_messages(session_file, true, lineage_entry_ids)
      local by_index = {}
      for _, m in ipairs(loaded.rendered) do by_index[m.index] = m end
      local invalid = {}
      for n in pairs(expand_ids) do if not by_index[n] then invalid[#invalid + 1] = n end end
      table.sort(invalid)
      if #invalid > 0 then
        return { content = { { type = "text", text = "Cannot expand indices outside " .. (scope == "all" and "session history" or "active lineage") .. ": " .. table.concat(invalid, ", ") } }, details = nil }
      end
      local expanded = {}
      for n in pairs(expand_ids) do if by_index[n] then expanded[#expanded + 1] = by_index[n] end end
      table.sort(expanded, function(a, b) return a.index < b.index end)
      local output = (scope == "all" and "Scope: all\n\n" or "") .. format_recall_output(expanded)
      return { content = { { type = "text", text = output } }, details = nil }
    end
    local loaded = load_all_messages(session_file, false, lineage_entry_ids)
    local all_results
    if params.query and params.query:gsub("%s", "") ~= "" then
      all_results = search_entries(loaded.rendered, loaded.rawMessages, params.query)
    else
      all_results = {}
      for i = math.max(1, #loaded.rendered - DEFAULT_RECENT + 1), #loaded.rendered do all_results[#all_results + 1] = loaded.rendered[i] end
    end
    if params.query and params.query:gsub("%s", "") ~= "" then
      local page = math.max(1, params.page or 1)
      local start = (page - 1) * PAGE_SIZE
      local page_results = {}
      for i = start + 1, math.min(#all_results, start + PAGE_SIZE) do page_results[#page_results + 1] = all_results[i] end
      local total_pages = math.max(1, math.ceil(#all_results / PAGE_SIZE))
      local scope_suffix = (scope == "all") and " (scope: all)" or ""
      local header = (total_pages > 1) and ("Page " .. page .. "/" .. total_pages .. " (" .. #all_results .. " total matches" .. scope_suffix .. ")") or (#all_results .. " matches" .. scope_suffix)
      local footer = (page < total_pages) and ("\n--- Use page:" .. (page + 1) .. (scope == "all" and " with scope:'all'" or "") .. " for more results ---") or ""
      local output = format_recall_output(page_results, params.query, header) .. footer
      return { content = { { type = "text", text = output } }, details = nil }
    end
    local output = (scope == "all" and "Scope: all\n\n" or "") .. format_recall_output(all_results, params.query)
    return { content = { { type = "text", text = output } }, details = nil }
  end,
  renderCall = function(args, theme)
    return { text = theme:fg("toolTitle", theme:bold("vcc_recall ")) .. theme:fg("muted", args.query or "(recent)") }
  end,
})

-- ── Entry ──────────────────────────────────────────────────────────
scaffold_settings()
register_before_compact_hook()
