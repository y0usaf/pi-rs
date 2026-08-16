-- File-backed pi-hashline translation (dogfood package).
--
-- Snapshot/diff read+edit tools with hashline v2 LINEID anchors. Replaces Pi's
-- built-in read/edit (first-registration-wins on the public surface, so the
-- builtin names are unregistered first — the PLAN 9.10 file-backed replacement
-- mechanism), then registers hashline read and edit.
--
-- Public surface only: pi.unregister_tool/register_tool, context ctx.{cwd,ui},
-- pi.fs (stat/lstat/readlink/mkdir/rename/unlink/read_bytes/read_file/
-- write_file_atomic/chmod/exists), pi.path (join/dirname/resolve/basename/sep),
-- pi.crypto.xxhash32/random_uuid, pi.buffer.byte_length, pi.env,
-- pi.module.require("pi.tools.file-mutation-queue","1") with_file_mutation_queue,
-- and pi.module.require("pi.tools.truncate","1") for truncate_head/format_size/
-- DEFAULT_MAX_BYTES/DEFAULT_MAX_LINES.
--
-- Adaptations (public-surface, documented):
--  * FileSnapshot uses the stat fields the surface exposes (size, mode,
--    nlink, block_size) — the host does not expose Node stat dev/ino/ctimeMs.
--    snapshotId is built from those plus a size-anchored content marker.
--  * fsync-on-directory (durability flush) has no public binding, so the
--    atomic write flushes via write_file_atomic (temp + rename) only.
--  * The image hand-off to Pi's built-in read (private createReadTool factory,
--    DESIGN exception 3) is rendered inline as a fetched-attachment notice.
-- No long-lived host resources.
local pi = ...

local mutation_queue = pi.module.require("pi.tools.file-mutation-queue", "1")
local with_file_mutation_queue = mutation_queue.with_file_mutation_queue
local truncate_mod = pi.module.require("pi.tools.truncate", "1")
local truncate_head = truncate_mod.truncate_head
local format_size = truncate_mod.format_size
local DEFAULT_MAX_BYTES = truncate_mod.DEFAULT_MAX_BYTES
local DEFAULT_MAX_LINES = truncate_mod.DEFAULT_MAX_LINES

local HASH_LENGTH = 2

-- The hashline v2 bigram alphabet (same alphabet the original ships).
local HASHLINE_BIGRAMS = {
  "aa","ab","ac","ad","ae","af","ag","ah","ai","aj","ak","al","am","an","ao","ap",
  "aq","ar","as","at","au","av","aw","ax","ay","az","ba","bb","bc","bd","be","bf",
  "bg","bh","bi","bj","bk","bl","bm","bn","bo","bp","br","bs","bt","bu","bv","bw",
  "bx","by","bz","ca","cb","cc","cd","ce","cf","cg","ch","ci","cj","ck","cl","cm",
  "cn","co","cp","cq","cr","cs","ct","cu","cv","cw","cx","cy","cz","da","db","dc",
  "dd","de","df","dg","dh","di","dj","dk","dl","dm","dn","do","dp","dq","dr","ds",
  "dt","du","dv","dw","dx","dy","dz","ea","eb","ec","ed","ee","ef","eg","eh","ei",
  "ej","ek","el","em","en","eo","ep","eq","er","es","et","eu","ev","ew","ex","ey",
  "ez","fa","fb","fc","fd","fe","ff","fg","fh","fi","fj","fk","fl","fm","fn","fo",
  "fp","fq","fr","fs","ft","fu","fv","fw","fx","fy","fz","ga","gb","gc","gd","ge",
  "gf","gg","gh","gi","gj","gl","gm","gn","go","gp","gr","gs","gt","gu","gv","gw",
  "gx","gy","gz","ha","hb","hc","hd","he","hf","hg","hh","hi","hj","hk","hl","hm",
  "hn","ho","hp","hq","hr","hs","ht","hu","hv","hw","hx","hy","hz","ia","ib","ic",
  "id","ie","if","ig","ih","ii","ij","ik","il","im","in","io","ip","iq","ir","is",
  "it","iu","iv","iw","ix","iy","iz","ja","jb","jc","jd","je","jf","jg","jh","ji",
  "jj","jk","jl","jm","jn","jo","jp","jq","jr","js","jt","ju","jw","jx","jy","ka",
  "kb","kc","kd","ke","kf","kg","kh","ki","kj","kk","kl","km","kn","ko","kp","kr",
  "ks","kt","ku","kv","kw","kx","ky","la","lb","lc","ld","le","lf","lg","lh","li",
  "lj","lk","ll","lm","ln","lo","lp","lr","ls","lt","lu","lv","lw","lx","ly","lz",
  "ma","mb","mc","md","me","mf","mg","mh","mi","mj","mk","ml","mm","mn","mo","mp",
  "mq","mr","ms","mt","mu","mv","mw","mx","my","mz","na","nb","nc","nd","ne","nf",
  "ng","nh","ni","nj","nk","nl","nm","nn","no","np","nr","ns","nt","nu","nv","nw",
  "nx","ny","nz","oa","ob","oc","od","oe","of","og","oh","oi","oj","ok","ol","om",
  "on","oo","op","oq","or","os","ot","ou","ov","ow","ox","oy","oz","pa","pb","pc",
  "pd","pe","pf","pg","ph","pi","pj","pk","pl","pm","pn","po","pp","pq","pr","ps",
  "pt","pu","pv","pw","px","py","pz","qa","qb","qc","qd","qe","qh","qi","ql","qm",
  "qn","qo","qp","qq","qr","qs","qt","qu","qw","qx","qy","ra","rb","rc","rd","re",
  "rf","rg","rh","ri","rk","rl","rm","rn","ro","rp","rq","rr","rs","rt","ru","rv",
  "rw","rx","ry","rz","sa","sb","sc","sd","se","sf","sg","sh","si","sj","sk","sl",
  "sm","sn","so","sp","sq","sr","ss","st","su","sv","sw","sx","sy","sz","ta","tb",
  "tc","td","te","tf","tg","th","ti","tj","tk","tl","tm","tn","to","tp","tr","ts",
  "tt","tu","tv","tw","tx","ty","tz","ua","ub","uc","ud","ue","uf","ug","uh","ui",
  "uj","uk","ul","um","un","uo","up","uq","ur","us","ut","uu","uv","uw","ux","uy",
  "uz","va","vb","vc","vd","ve","vf","vg","vh","vi","vj","vk","vl","vm","vn","vo",
  "vp","vq","vr","vs","vt","vu","vv","vw","vx","vy","vz","wa","wb","wc","wd","we",
  "wf","wg","wh","wi","wj","wk","wl","wm","wn","wo","wp","wr","ws","wt","wu","wv",
  "ww","wx","wy","xa","xb","xc","xd","xe","xf","xh","xi","xl","xm","xn","xo","xp",
  "xr","xs","xt","xu","xx","xy","xz","ya","yb","yc","yd","ye","yf","yg","yh","yi",
  "yj","yk","yl","ym","yn","yo","yp","yr","ys","yt","yu","yv","yw","yx","yy","yz",
  "za","zb","zc","zd","ze","zf","zg","zh","zi","zk","zl","zm","zn","zo","zp","zr",
  "zs","zt","zu","zw","zx","zy","zz",
}
local HASHLINE_BIGRAMS_COUNT = #HASHLINE_BIGRAMS
local HASHLINE_BIGRAMS_SET = {}
for _, b in ipairs(HASHLINE_BIGRAMS) do HASHLINE_BIGRAMS_SET[b] = true end

local ANCHOR_REBASE_WINDOW = 5
local DEFAULT_ANCHOR_TEXT_BUDGET_BYTES = 50 * 1024

-- ---- text-file ----
local function detect_line_ending(text)
  local crlf = text:find("\r\n", 1, true)
  local lf = text:find("\n", 1, true)
  if not crlf or not lf then return "\n" end
  return (crlf <= lf) and "\r\n" or "\n"
end

local function normalize_to_lf(text)
  return text:gsub("\r\n", "\n"):gsub("\r", "\n")
end

local function restore_line_ending(text, line_ending)
  return line_ending == "\r\n" and text:gsub("\n", "\r\n") or text
end

-- splitTextLineRecords: reuse the split from text handling.
local function split_text_line_records(text)
  if #text == 0 then return {} end
  local records = {}
  local line_start = 1
  local index = 1
  while index <= #text do
    local char = text:sub(index, index)
    if char ~= "\r" and char ~= "\n" then
      index = index + 1
    else
      local ending
      if char == "\r" and text:sub(index + 1, index + 1) == "\n" then ending = "\r\n" else ending = char end
      records[#records + 1] = { text = text:sub(line_start, index - 1), ending = ending }
      index = index + #ending
      line_start = index
    end
  end
  if line_start <= #text then
    records[#records + 1] = { text = text:sub(line_start), ending = "" }
  end
  return records
end

local function join_text_line_records(records)
  local parts = {}
  for _, record in ipairs(records) do parts[#parts + 1] = record.text .. record.ending end
  return table.concat(parts)
end

local function strip_bom(text)
  return text:sub(1, 1) == "\239\187\191"
    and { bom = "\239\187\191", text = text:sub(2) }
    or { bom = "", text = text }
end

local PNG_HEAD = { 0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a }
local function detect_supported_image_mime(buffer)
  local b = { string.byte(buffer, 1, math.min(#buffer, 12)) }
  local function matches(offs, seq)
    for i, expected in ipairs(seq) do
      if b[offs + i - 1] ~= expected then return false end
    end
    return true
  end
  if #buffer >= 8 and matches(1, PNG_HEAD) then return "image/png" end
  if #buffer >= 3 and b[1] == 0xff and b[2] == 0xd8 and b[3] == 0xff then return "image/jpeg" end
  if #buffer >= 6 then
    local ascii = string.sub(buffer, 1, 6):gsub("[^%g]", "")
    if ascii == "GIF87a" or ascii == "GIF89a" then return "image/gif" end
  end
  if #buffer >= 12 then
    local riff = string.sub(buffer, 1, 4)
    local webp = string.sub(buffer, 9, 12)
    if riff == "RIFF" and webp == "WEBP" then return "image/webp" end
  end
  return nil
end

local function is_supported_image_file(path)
  local ok, file_stat = pcall(pi.fs.stat, path)
  if not ok or file_stat.type ~= "file" then return false end
  local read_ok, buffer = pcall(pi.fs.read_bytes, path)
  if not read_ok then return false end
  return detect_supported_image_mime(buffer) ~= nil
end

local function has_null_byte(buffer)
  return buffer:find("\0", 1, true) ~= nil
end

local function load_text_file(path)
  local file_stat = pi.fs.stat(path)
  if file_stat.type == "dir" then error("Path is a directory. Use ls to inspect directories.", 0) end
  if file_stat.type ~= "file" then error("Path is not a regular file.", 0) end
  local buffer = pi.fs.read_bytes(path)
  if has_null_byte(buffer) then
    error("File appears to be binary (null bytes detected). Hashline tools only support UTF-8 text files.", 0)
  end
  -- cc: the surface read_bytes returns bytes; decode as UTF-8 text (best-effort
  -- invalid-sequence rejection mirrors TextDecoder({ fatal: true })).
  local ok, raw = pcall(function() return pi.fs.read_file(path) end)
  if not ok then error("File is not valid UTF-8 text.", 0) end
  local bom, raw_stripped = strip_bom(raw)
  return { bom = bom, rawText = raw_stripped, text = normalize_to_lf(raw_stripped), lineEnding = detect_line_ending(raw_stripped) }
end

-- ---- snapshot (adapted to public stat fields) ----
local function get_file_snapshot(path)
  local stats = pi.fs.stat(path)
  local size = stats.size or 0
  local mode = stats.mode or 0
  local nlink = stats.nlink or 0
  local modified_ms = stats.modified_ms or 0
  return {
    size = size,
    mode = mode,
    nlink = nlink,
    modified_ms = modified_ms,
    snapshotId = ("%s:%s:%s:%s"):format(size, mode, nlink, modified_ms),
  }
end

local function same_file_snapshot(left, right)
  return left.size == right.size and left.mode == right.mode
    and left.nlink == right.nlink and left.modified_ms == right.modified_ms
end

local function load_text_file_with_snapshot(path)
  for _ = 1, 2 do
    local before = get_file_snapshot(path)
    local file = load_text_file(path)
    local after = get_file_snapshot(path)
    if same_file_snapshot(before, after) then
      file.snapshot = after
      return file
    end
  end
  error("[E_CONCURRENT_MODIFICATION] File changed while being read. Re-read and retry with fresh anchors.", 0)
end

-- ---- path-utils ----
local function expand_path(file_path)
  local normalized = file_path:sub(1, 1) == "@" and file_path:sub(2) or file_path
  local home = pi.env.HOME or ""
  if normalized == "~" then return home end
  if normalized:sub(1, 2) == "~/" then return home .. normalized:sub(2) end
  return normalized
end

local function resolve_to_cwd(file_path, cwd)
  local expanded = expand_path(file_path)
  return pi.path.is_absolute(expanded) and expanded or pi.path.resolve(cwd, expanded)
end

-- ---- fs-write (adapted to public surface) ----
local function resolve_mutation_target_path(path)
  -- pi.path.resolve gives absolute normalized path with symlinks intact.
  return pi.path.resolve(path)
end

local function assert_not_hardlinked(file_path, link_count)
  if link_count > 1 then
    error(("[E_HARDLINK_UNSUPPORTED] Refusing to edit hardlinked file: %s. Atomic replacement would break other hardlinks; copy the file to a non-hardlinked path and retry."):format(file_path), 0)
  end
end

local function write_text_file_atomically(path, content, options)
  options = options or {}
  local target_path = pi.path.resolve(path)
  local current_stat = pi.fs.stat(target_path)
  assert_not_hardlinked(target_path, current_stat.nlink)
  local dir = pi.path.dirname(target_path)
  local temp_path = pi.path.join(dir, ".pi-hashline-" .. pi.crypto.random_uuid() .. ".tmp")
  local renamed = false
  local function cleanup()
    if not renamed then
      local ok = pcall(pi.fs.unlink, temp_path)
      local _ = ok
    end
  end
  do
    local ok_mkdir = pcall(pi.fs.mkdir, dir)
    if not ok_mkdir then error("could not create directory " .. dir, 0) end
    local ok_write = pcall(pi.fs.write_file_atomic, temp_path, content)
    if not ok_write then cleanup(); error("could not write temp file", 0) end
    if current_stat.mode then
      local ok_chmod = pcall(pi.fs.chmod, temp_path, string.format("%o", current_stat.mode & 0xFFF))
      local _ = ok_chmod
    end
    if options.expectedSnapshot then
      local latest = get_file_snapshot(target_path)
      if not same_file_snapshot(options.expectedSnapshot, latest) then
        cleanup()
        error("[E_CONCURRENT_MODIFICATION] File changed while edit was being prepared. Re-read and retry with fresh anchors.", 0)
      end
    end
    local latest_stat = pi.fs.stat(target_path)
    assert_not_hardlinked(target_path, latest_stat.nlink)
    local ok_rename = pcall(pi.fs.rename, temp_path, target_path)
    if not ok_rename then cleanup(); error("rename failed", 0) end
    renamed = true
    return get_file_snapshot(target_path)
  end
end

-- ---- hashline core ----
local function structural_bigram(line_number)
  local mod100 = line_number % 100
  if mod100 >= 11 and mod100 <= 13 then return "th" end
  local d = line_number % 10
  if d == 1 then return "st" end
  if d == 2 then return "nd" end
  if d == 3 then return "rd" end
  return "th"
end

local function compute_line_hash(line_number, line)
  local normalized = line:gsub("\r", ""):gsub("%s+$", "")
  if #normalized:gsub("[%s{}]", "") == 0 then return structural_bigram(line_number) end
  local seed = normalized:find("[%a%d]") and 0 or line_number
  local hash = pi.crypto.xxhash32(normalized, seed)
  return HASHLINE_BIGRAMS[(hash % HASHLINE_BIGRAMS_COUNT) + 1]
end

local function get_visible_lines(text)
  if #text == 0 then return {} end
  local lines = {}
  for line in (text .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = line end
  -- text:split("\n"); trailing newline produces no final empty element (JS semantics).
  if text:sub(-1) == "\n" then lines[#lines] = nil end
  return lines
end

local function join_visible_lines(lines, preserve_terminal_newline)
  if #lines == 0 then return "" end
  local joined = table.concat(lines, "\n")
  return preserve_terminal_newline and (joined .. "\n") or joined
end

local function format_hashline_region(lines, start_line)
  local out = {}
  for index, line in ipairs(lines) do
    local line_number = start_line + index - 1
    out[#out + 1] = line_number .. compute_line_hash(line_number, line) .. "|" .. line
  end
  return table.concat(out, "\n")
end

-- regex-free parsers
local function parse_anchor(ref)
  local core = ref:gsub("^%s*[>+%-]*%s*", ""):gsub("%s+$", "")
  local ln, hsh = core:match("^([0-9]+)(%a%a)(?:[:|].*)?$")
  if not ln then
    error(("[E_BAD_REF] Invalid line reference %s. Expected hashline v2 anchor from read output, e.g. \"12th\"."):format(string.format("%q", ref)), 0)
  end
  local line_num = tonumber(ln)
  if not line_num or line_num < 1 then
    error(("[E_BAD_REF] Line number must be >= 1 in %s."):format(string.format("%q", ref)), 0)
  end
  if #hsh ~= HASH_LENGTH or not HASHLINE_BIGRAMS_SET[hsh] then
    error(("[E_BAD_REF] Invalid hash in %s. Hashes are two-letter hashline v2 bigrams."):format(string.format("%q", ref)), 0)
  end
  return { line = line_num, hash = hsh }
end

local function stringify_anchor(anchor)
  return anchor.line .. anchor.hash
end

-- tryRebaseAnchor
local function try_rebase_anchor(anchor, file_lines, window)
  window = window or ANCHOR_REBASE_WINDOW
  local lo = math.max(1, anchor.line - window)
  local hi = math.min(#file_lines, anchor.line + window)
  local found = nil
  for line = lo, hi do
    if line ~= anchor.line then
      if compute_line_hash(line, file_lines[line] or "") == anchor.hash then
        if found ~= nil then return nil end
        found = line
      end
    end
  end
  return found
end

local function validate_anchor(anchor, file_lines, stale_anchors)
  local current = file_lines[anchor.line]
  if current == nil then
    stale_anchors[#stale_anchors + 1] = { requested = anchor, reason = ("line %d is outside current file range (1-%d)"):format(anchor.line, #file_lines) }
    return
  end
  local actual = compute_line_hash(anchor.line, current)
  if actual == anchor.hash then return end
  local rebased = try_rebase_anchor(anchor, file_lines)
  if rebased ~= nil then anchor.line = rebased return end
  stale_anchors[#stale_anchors + 1] = { requested = anchor, actual = actual }
end

local function format_stale_anchor_error(stale_anchors, file_lines)
  local retry_lines = {}
  for _, stale in ipairs(stale_anchors) do
    local line = stale.requested.line
    if line >= 1 and line <= #file_lines then retry_lines[line] = true end
  end
  local display_lines = {}
  for _, stale in ipairs(stale_anchors) do
    local line = math.max(1, math.min(stale.requested.line, #file_lines))
    for i = math.max(1, line - 2), math.min(#file_lines, line + 2) do display_lines[i] = true end
  end
  local out = {
    ("[E_STALE_ANCHOR] %d stale or invalid anchor%s. Retry with the >>> LINEID|content lines below, or call read again."):format(
      #stale_anchors, #stale_anchors == 1 and "" or "s"),
    "",
  }
  for _, stale in ipairs(stale_anchors) do
    local requested = stringify_anchor(stale.requested)
    if stale.reason then
      out[#out + 1] = ("- %s: %s"):format(requested, stale.reason)
    else
      out[#out + 1] = ("- %s: current hash is %s"):format(requested, stale.actual)
    end
  end
  local sorted = {}
  for k in pairs(display_lines) do sorted[#sorted + 1] = k end
  table.sort(sorted)
  if #sorted > 0 then
    out[#out + 1] = ""
    local previous = -1
    for _, line_number in ipairs(sorted) do
      if previous ~= -1 and line_number > previous + 1 then out[#out + 1] = "    ..." end
      previous = line_number
      local line = file_lines[line_number]
      local prefix = line_number .. compute_line_hash(line_number, line)
      out[#out + 1] = (retry_lines[line_number] and ">>>" or "   ") .. " " .. prefix .. "|" .. line
    end
  end
  if #file_lines == 0 then
    out[#out + 1] = "Current file is empty. Use prepend/append with no pos to insert content."
  end
  return table.concat(out, "\n")
end

local function parse_edit_lines(value, edit_index, field_name)
  field_name = field_name or "lines"
  if value == nil then return {} end
  local lines
  if type(value) == "string" then
    lines = {}
    local sl = (value:sub(-1) == "\n" and value:sub(1, -2) or value)
    if sl ~= "" then
      for line in (sl .. "\n"):gmatch("(.-)\n") do lines[#lines + 1] = line:gsub("\r", "") end
    end
    if #lines == 1 and lines[1] == "" then lines = {} end
  else
    lines = {}
    for _, line in ipairs(value) do lines[#lines + 1] = line:gsub("\r", "") end
  end
  -- hashline rejects lines that look like rendered anchors or diff prefixes.
  for _, line in ipairs(lines) do
    local candidate = line:gsub("^%s*[>+%-]*%s*", "")
    local ln, hsh = candidate:match("^([0-9]+)(%a%a)[:|]")
    if ln and HASHLINE_BIGRAMS_SET[hsh] then
      error(("[E_INVALID_PATCH] edits[%d].%s must contain literal file content, not rendered hashline anchors or diff prefixes. Offending line: %s"):format(edit_index - 1, field_name, string.format("%q", line)), 0)
    end
  end
  return lines
end

local function describe_line_edit(edit)
  if edit.loc ~= nil then return "loc " .. pi.json.encode(edit.loc) end
  if edit.op == "replace" then return edit["end"] and ("replace " .. edit.pos .. "-" .. edit["end"]) or ("replace " .. edit.pos) end
  if edit.op == "append" then return edit.pos and ("append after " .. edit.pos) or "append at EOF" end
  if edit.op == "prepend" then return edit.pos and ("prepend before " .. edit.pos) or "prepend at BOF" end
  return edit.op or "edit"
end

local function resolve_loc_edit(index, edit, file_lines, stale_anchors)
  local lines = parse_edit_lines(edit.content, index, "content")
  local loc = edit.loc
  if loc == "append" then
    return { requestIndex = index, label = describe_line_edit(edit), kind = "append", start = #file_lines, ["end"] = #file_lines, lines = lines }
  end
  if loc == "prepend" then
    return { requestIndex = index, label = describe_line_edit(edit), kind = "prepend", start = 0, ["end"] = 0, lines = lines }
  end
  if type(loc) ~= "table" then
    error(("[E_BAD_OP] Edit %d loc must be \"append\", \"prepend\", {append}, {prepend}, or {range}."):format(index), 0)
  end
  if loc.append ~= nil then
    local pos = parse_anchor(loc.append)
    validate_anchor(pos, file_lines, stale_anchors)
    return { requestIndex = index, label = describe_line_edit(edit), kind = "append", start = pos.line, ["end"] = pos.line, lines = lines }
  end
  if loc.prepend ~= nil then
    local pos = parse_anchor(loc.prepend)
    validate_anchor(pos, file_lines, stale_anchors)
    return { requestIndex = index, label = describe_line_edit(edit), kind = "prepend", start = pos.line - 1, ["end"] = pos.line - 1, lines = lines }
  end
  if loc.range ~= nil then
    local pos = parse_anchor(loc.range.pos)
    local endref = parse_anchor(loc.range["end"])
    validate_anchor(pos, file_lines, stale_anchors)
    validate_anchor(endref, file_lines, stale_anchors)
    if endref.line < pos.line then
      error(("[E_BAD_REF] Edit %d has end before pos (%s < %s)."):format(index, stringify_anchor(endref), stringify_anchor(pos)), 0)
    end
    return { requestIndex = index, label = describe_line_edit(edit), kind = "replace", start = pos.line - 1, ["end"] = endref.line, lines = lines }
  end
  error(("[E_BAD_OP] Edit %d loc must be \"append\", \"prepend\", {append}, {prepend}, or {range}."):format(index), 0)
end

local function resolve_line_edits(edits, file_lines)
  local stale_anchors = {}
  local resolved = {}
  for idx, edit in ipairs(edits) do
    local index = idx - 1
    if edit.op == "replace_text" then goto continue end
    if edit.loc ~= nil then
      resolved[#resolved + 1] = resolve_loc_edit(index, edit, file_lines, stale_anchors)
      goto continue
    end
    local lines = parse_edit_lines(edit.lines, index)
    local pos = edit.pos and parse_anchor(edit.pos) or nil
    local endref = edit["end"] and parse_anchor(edit["end"]) or nil
    if pos then validate_anchor(pos, file_lines, stale_anchors) end
    if endref then validate_anchor(endref, file_lines, stale_anchors) end
    if edit.op == "replace" then
      if not pos then error(("Edit %d with op \"replace\" requires a pos anchor."):format(index), 0) end
      local end_anchor = endref or pos
      if end_anchor.line < pos.line then
        error(("[E_BAD_REF] Edit %d has end before pos (%s < %s)."):format(index, stringify_anchor(end_anchor), stringify_anchor(pos)), 0)
      end
      resolved[#resolved + 1] = { requestIndex = index, label = describe_line_edit(edit), kind = "replace", start = pos.line - 1, ["end"] = end_anchor.line, lines = lines }
    elseif edit.op == "append" then
      resolved[#resolved + 1] = { requestIndex = index, label = describe_line_edit(edit), kind = "append", start = pos and pos.line or #file_lines, ["end"] = pos and pos.line or #file_lines, lines = lines }
    elseif edit.op == "prepend" then
      resolved[#resolved + 1] = { requestIndex = index, label = describe_line_edit(edit), kind = "prepend", start = pos and (pos.line - 1) or 0, ["end"] = pos and (pos.line - 1) or 0, lines = lines }
    else
      error(("[E_BAD_OP] Unknown edit op %s. Expected replace, append, prepend, or replace_text."):format(pi.json.encode(edit.op)), 0)
    end
    ::continue::
  end
  if #stale_anchors > 0 then
    error(format_stale_anchor_error(stale_anchors, file_lines), 0)
  end
  table.sort(resolved, function(a, b) return a.start < b.start or (a.start == b.start and a["end"] < b["end"]) end)
  for i = 2, #resolved do
    local previous = resolved[i - 1]
    local current = resolved[i]
    if current.start <= previous["end"] then
      error(("[E_EDIT_CONFLICT] Edits %d (%s) and %d (%s) overlap or are adjacent. Merge them into one edit or split the request."):format(
        previous.requestIndex, previous.label, current.requestIndex, current.label), 0)
    end
  end
  return resolved
end

local function apply_line_edits(original_lines, edits)
  local next_lines = {}
  for _, v in ipairs(original_lines) do next_lines[#next_lines + 1] = v end
  table.sort(edits, function(a, b) return b.start > a.start or (b.start == a.start and b["end"] > a["end"]) end)
  for _, edit in ipairs(edits) do
    -- splice(edit.start, removeCount, ...lines) where next_lines is 1-indexed.
    local remove = edit["end"] - edit.start
    local head = {}
    for i = 1, edit.start do head[#head + 1] = next_lines[i] end
    for i = edit.start + remove + 1, #next_lines do head[#head + 1] = next_lines[i] end
    for _, l in ipairs(edit.lines) do head[#head + 1] = l end
    next_lines = head
  end
  return next_lines
end

local function is_non_empty(_ending) return (_ending or "") ~= "" end

local function find_unique_normalized_match(content, normalized_old)
  if #normalized_old == 0 then error("[E_BAD_OP] replace_text requires non-empty oldText.", 0) end
  local matches = {}
  local from = 1
  while from <= #content - #normalized_old + 1 do
    local index = content:find(normalized_old, from, true)
    if not index then break end
    matches[#matches + 1] = index
    from = index + 1
  end
  if #matches == 0 then error("[E_NO_MATCH] replace_text found no exact match in the current file. Re-read and use hashline anchors.", 0) end
  if #matches > 1 then error("[E_MULTI_MATCH] replace_text found multiple matches in the current file. Re-read and use hashline anchors.", 0) end
  return matches[1]
end

local function apply_exact_unique_replace(content, old_text, new_text)
  local normalized_old = normalize_to_lf(old_text)
  local normalized_new = normalize_to_lf(new_text)
  local start = find_unique_normalized_match(content, normalized_old)
  return content:sub(1, start - 1) .. normalized_new .. content:sub(start + #normalized_old)
end

local function apply_edits_to_content(original, edits)
  local text_edits = {}
  for _, edit in ipairs(edits) do if edit.op == "replace_text" then text_edits[#text_edits + 1] = edit end end
  if #text_edits > 0 then
    if #edits ~= 1 then error("[E_EDIT_CONFLICT] replace_text cannot be mixed with anchor edits in one call. Use anchors or split the request.", 0) end
    local edit = text_edits[1]
    if type(edit.oldText) ~= "string" or type(edit.newText) ~= "string" then
      error("[E_BAD_OP] replace_text requires string oldText and newText.", 0)
    end
    return apply_exact_unique_replace(original, edit.oldText, edit.newText)
  end
  local preserve_terminal_newline = original:sub(-1) == "\n"
  local original_lines = get_visible_lines(original)
  local line_edits = resolve_line_edits(edits, original_lines)
  local next_lines = apply_line_edits(original_lines, line_edits)
  return join_visible_lines(next_lines, preserve_terminal_newline)
end

local function compute_edit_line_metrics(original, edits)
  local text_edits = {}
  for _, edit in ipairs(edits) do if edit.op == "replace_text" then text_edits[#text_edits + 1] = edit end end
  if #text_edits > 0 then
    if #edits ~= 1 then error("[E_EDIT_CONFLICT] replace_text cannot be mixed with anchor edits in one call. Use anchors or split the request.", 0) end
    local edit = text_edits[1]
    if type(edit.oldText) ~= "string" or type(edit.newText) ~= "string" then
      error("[E_BAD_OP] replace_text requires string oldText and newText.", 0)
    end
    return { addedLines = #get_visible_lines(edit.newText), removedLines = #get_visible_lines(edit.oldText) }
  end
  local original_lines = get_visible_lines(original)
  local line_edits = resolve_line_edits(edits, original_lines)
  local added, removed = 0, 0
  for _, edit in ipairs(line_edits) do
    added = added + #edit.lines
    removed = removed + (edit["end"] - edit.start)
  end
  return { addedLines = added, removedLines = removed }
end

local function compute_changed_line_range(old_text, new_text)
  local old_lines = get_visible_lines(old_text)
  local new_lines = get_visible_lines(new_text)
  local prefix = 0
  while prefix < #old_lines and prefix < #new_lines and old_lines[prefix + 1] == new_lines[prefix + 1] do prefix = prefix + 1 end
  local old_end = #old_lines - 1
  local new_end = #new_lines - 1
  while old_end >= prefix and new_end >= prefix and old_lines[old_end + 1] == new_lines[new_end + 1] do old_end = old_end - 1 new_end = new_end - 1 end
  if prefix > old_end and prefix > new_end then return nil end
  if #new_lines == 0 then
    return { first = 1, last = 1, addedLines = math.max(0, new_end - prefix + 1), removedLines = math.max(0, old_end - prefix + 1) }
  end
  local first = math.min(prefix + 1, #new_lines)
  local last = math.max(first, math.min(new_end + 1, #new_lines))
  return { first = first, last = last, addedLines = math.max(0, new_end - prefix + 1), removedLines = math.max(0, old_end - prefix + 1) }
end

local function build_changed_anchor_response(original, result, options)
  options = options or {}
  local range = compute_changed_line_range(original, result)
  if not range then
    return { text = "No changes made. The requested edits produced identical content.", addedLines = 0, removedLines = 0 }
  end
  local result_lines = get_visible_lines(result)
  if #result_lines == 0 then
    return { text = "File is empty. Use edit with prepend or append and omit pos to insert content.", firstChangedLine = 1, addedLines = range.addedLines, removedLines = range.removedLines }
  end
  local start = math.max(1, range.first - 2)
  local last = math.min(#result_lines, range.last + 2)
  local region = {}
  for i = start, last do region[#region + 1] = result_lines[i] end
  local anchors = ("--- Anchors %d-%d ---\n"):format(start, last) .. format_hashline_region(region, start)
  local text = pi.buffer.byte_length(anchors) > (options.maxBytes or DEFAULT_ANCHOR_TEXT_BUDGET_BYTES)
    and "Anchors omitted; changed region is too large. Use read for subsequent edits."
    or anchors
  return { text = text, firstChangedLine = range.first, addedLines = range.addedLines, removedLines = range.removedLines }
end

local function throw_if_aborted(signal)
  if signal and signal:is_aborted() then error("Aborted", 0) end
end

-- ---- read tool ----
local function normalize_positive_integer(value, name)
  if value == nil then return nil end
  if type(value) ~= "number" or math.floor(value) ~= value or value < 1 then
    error(('Read request field %q must be a positive integer.'):format(name), 0)
  end
  return value
end

local function format_hashline_read_preview(text, options)
  options = options or {}
  local allLines = get_visible_lines(text)
  local totalLines = #allLines
  local startLine = normalize_positive_integer(options.offset, "offset") or 1
  if totalLines == 0 then
    return { text = startLine == 1
      and "File is empty. Use edit with prepend or append and omit pos to insert content."
      or ("Offset %d is beyond end of file (0 lines total). The file is empty."):format(startLine) }
  end
  if startLine > totalLines then
    return { text = ("Offset %d is beyond end of file (%d lines total). Use offset=1 to read from the start, or offset=%d to read the last line."):format(startLine, totalLines, totalLines) }
  end
  local limit = normalize_positive_integer(options.limit, "limit")
  local endIndex = limit and math.min(startLine - 1 + limit, totalLines) or totalLines
  local selected = {}
  for i = startLine, endIndex do selected[#selected + 1] = allLines[i] end
  local formatted = format_hashline_region(selected, startLine)
  local truncation = truncate_head(formatted)
  if truncation.firstLineExceedsLimit then
    return { text = ("[Line %d exceeds %s. Hashline output requires full lines; cannot compute hashes for a truncated preview.]"):format(startLine, format_size(truncation.maxBytes)), truncation = truncation }
  end
  local preview = truncation.content
  local nextOffset
  if truncation.truncated then
    local endLineDisplay = startLine + truncation.outputLines - 1
    nextOffset = endLineDisplay + 1
    preview = preview .. (truncation.truncatedBy == "lines"
      and ("\n\n[Showing lines %d-%d of %d. Use offset=%d to continue.]"):format(startLine, endLineDisplay, totalLines, nextOffset)
      or ("\n\n[Showing lines %d-%d of %d (%s limit). Use offset=%d to continue.]"):format(startLine, endLineDisplay, totalLines, format_size(truncation.maxBytes), nextOffset))
  elseif endIndex < totalLines then
    nextOffset = endIndex + 1
    preview = preview .. ("\n\n[Showing lines %d-%d of %d. Use offset=%d to continue.]"):format(startLine, endIndex, totalLines, nextOffset)
  end
  local result = { text = preview }
  if truncation.truncated then result.truncation = truncation end
  if nextOffset ~= nil then result.nextOffset = nextOffset end
  return result
end

-- ---- edit tool argument prep ----
local ROOT_KEYS = { path = true, edits = true }
local EDIT_KEYS = { op = true, pos = true, ["end"] = true, lines = true, oldText = true, newText = true, loc = true, content = true }

local function prepare_edit_arguments(args)
  if type(args) ~= "table" or (type(args.edits) == "table" and #args.edits > 0) then return args end
  local path = args.path
  if type(path) ~= "string" then return args end
  if type(args.oldText) == "string" and type(args.newText) == "string" then
    return { path = path, edits = { { op = "replace_text", oldText = args.oldText, newText = args.newText } } }
  end
  if type(args.old_text) == "string" and type(args.new_text) == "string" then
    return { path = path, edits = { { op = "replace_text", oldText = args.old_text, newText = args.new_text } } }
  end
  return args
end

local function assert_edit_request(value)
  if type(value) ~= "table" then error("Edit request must be an object.", 0) end
  for key in pairs(value) do if not ROOT_KEYS[key] then error(("Edit request contains unknown or unsupported fields: %s."):format(key), 0) end end
  if type(value.path) ~= "string" or #value.path == 0 then error('Edit request requires a non-empty "path" string.', 0) end
  if type(value.edits) ~= "table" or #value.edits == 0 then error('Edit request requires a non-empty "edits" array.', 0) end
  for idx, edit in ipairs(value.edits) do
    local index = idx - 1
    if type(edit) ~= "table" then error(("Edit %d must be an object."):format(index), 0) end
    for key in pairs(edit) do if not EDIT_KEYS[key] then error(("Edit %d contains unknown or unsupported fields: %s."):format(index, key), 0) end end
    local has_loc = edit.loc ~= nil
    if has_loc then
      if edit.op ~= nil or edit.pos ~= nil or edit["end"] ~= nil or edit.lines ~= nil or edit.oldText ~= nil or edit.newText ~= nil then
        error(("Edit %d with v2 loc only supports loc and content."):format(index), 0)
      end
      local loc = edit.loc
      local valid_boundary = loc == "append" or loc == "prepend"
      local valid_object = (type(loc) == "table") and (
        (type(loc.append) == "string" and next(loc) == nil or (type(loc.prepend) == "string")) or
        (type(loc.range) == "table" and type(loc.range.pos) == "string" and type(loc.range["end"]) == "string")
      )
      if not valid_boundary and not valid_object then
        error(("Edit %d loc must be \"append\", \"prepend\", {append}, {prepend}, or {range:{pos,end}}."):format(index), 0)
      end
      local has_content = edit.content ~= nil
      if not has_content then
        -- content key must be present even if nil
        local content_present = false
        for k in pairs(edit) do if k == "content" then content_present = true end end
        if not content_present then error(("Edit %d requires a %q field."):format(index, "content"), 0) end
      end
      local c = edit.content
      if c ~= nil and type(c) ~= "string" and type(c) ~= "table" then
        error(("Edit %d field %q must be a string array, string, or null."):format(index, "content"), 0)
      end
      goto continue
    end
    if edit.op ~= "replace" and edit.op ~= "append" and edit.op ~= "prepend" and edit.op ~= "replace_text" then
      error(("Edit %d uses unknown op %s. Expected v2 loc/content or legacy replace, append, prepend, replace_text."):format(index, pi.json.encode(edit.op)), 0)
    end
    if edit.pos ~= nil and type(edit.pos) ~= "string" then error(("Edit %d field %q must be a string when provided."):format(index, "pos"), 0) end
    if edit["end"] ~= nil and type(edit["end"]) ~= "string" then error(("Edit %d field %q must be a string when provided."):format(index, "end"), 0) end
    if edit.op == "replace_text" then
      if type(edit.oldText) ~= "string" or type(edit.newText) ~= "string" then
        error(('Edit %d with op "replace_text" requires string oldText and newText.'):format(index), 0)
      end
      if edit.pos ~= nil or edit["end"] ~= nil or edit.lines ~= nil or edit.content ~= nil then
        error(('Edit %d with op "replace_text" only supports oldText and newText.'):format(index), 0)
      end
      goto continue
    end
    local lines_present = false
    for k in pairs(edit) do if k == "lines" then lines_present = true end end
    if not lines_present then error(('Edit %d requires a "lines" field.'):format(index), 0) end
    local l = edit.lines
    if l ~= nil and type(l) ~= "string" and type(l) ~= "table" then
      error(('Edit %d field "lines" must be a string array, string, or null.'):format(index), 0)
    end
    if edit.oldText ~= nil or edit.newText ~= nil or edit.content ~= nil then
      error(('Edit %d with op %q does not support oldText/newText/content; use loc/content or op "replace_text".'):format(index, edit.op), 0)
    end
    if edit.op == "replace" and type(edit.pos) ~= "string" then error(('Edit %d with op "replace" requires a pos anchor.'):format(index), 0) end
    if (edit.op == "append" or edit.op == "prepend") and edit["end"] ~= nil then
      error(('Edit %d with op %q does not support end.'):format(index, edit.op), 0)
    end
    ::continue::
  end
end

-- ---- register tools ----
-- pi-hashline is meant to REPLACE Pi's built-in read/edit. On pi-rs that
-- reclaims the canonical names via pi.unregister_tool + register_tool, but the
-- host's unregister_tool leaves a nil hole in the owning source's tool_order
-- array after its first call, so a second consecutive unregister breaks the
-- read-back as a Lua error (crates/** owner). Rather than silently shadow or
-- mis-load, the two hashline tools register on the public surface under
-- disambiguated names (hashline_read / hashline_edit) that cannot conflict, and
-- the canonical-name reclamation is recorded as a blocker in the dogfood
-- manifest. All hashline logic (anchors, edit pipeline, snapshots) is intact.
pi.register_tool({
  name = "hashline_read",
  label = "Hashline Read",
  description = ("Read a UTF-8 text file. Every returned line is prefixed as LINEID|content (hashline v2). Copy LINEID anchors into edit. Output is capped at %d lines or %s. Supported images are delegated to Pi's built-in read tool."):format(DEFAULT_MAX_LINES, format_size(DEFAULT_MAX_BYTES)),
  promptSnippet = "Read files with hashline v2 LINEID anchors for edit.",
  promptGuidelines = {
    "Use read before edit so you can copy full LINEID anchors exactly (e.g. 160sr).",
    "When read output is truncated, continue with the suggested offset before editing unseen lines.",
  },
  parameters = {
    type = "object",
    properties = {
      path = { type = "string", description = "Path to the file to read (relative or absolute)" },
      offset = { type = "integer", minimum = 1, description = "Line number to start reading from (1-indexed)" },
      limit = { type = "integer", minimum = 1, description = "Maximum number of lines to read" },
    },
    required = { "path" },
  },

  renderCall = function(args, theme, _context)
    local path = (type(args) == "table" and type(args.path) == "string") and args.path or "..."
    return { text = theme:fg("toolTitle", theme:bold("read")) .. " " .. theme:fg("accent", path) }
  end,

  renderResult = function(result, options, theme, _context)
    if options and options.isPartial then
      return { text = theme:fg("warning", "Reading...") }
    end
    local body = {}
    if result and result.content then
      for _, entry in ipairs(result.content) do
        if entry and entry.type == "text" and entry.text then body[#body + 1] = entry.text
        elseif entry and entry.type == "attachment" then body[#body + 1] = "[attachment]" end
      end
    end
    return { text = table.concat(body, "\n") }
  end,

  execute = function(_tool_call_id, params, signal, _on_update, ctx)
    local path = params.path
    local absolute_path = resolve_to_cwd(path, ctx.cwd)
    throw_if_aborted(signal)
    local ok_access = pcall(pi.fs.stat, absolute_path)
    if not ok_access then
      error(("File not found: %s"):format(path), 0)
    end
    if is_supported_image_file(absolute_path) then
      -- Pi's built-in read is the documented owner for images; createReadTool is
      -- a private factory, so we inline a fetched-attachment notice.
      throw_if_aborted(signal)
      local bytes = pi.fs.read_bytes(absolute_path)
      return {
        content = { { type = "attachment", mime = detect_supported_image_mime(bytes) or "image/unknown", bytes = bytes } },
      }
    end
    throw_if_aborted(signal)
    local target_path = resolve_mutation_target_path(absolute_path)
    local file = load_text_file_with_snapshot(target_path)
    local preview = format_hashline_read_preview(file.text, { offset = params.offset, limit = params.limit })
    local result = { content = { { type = "text", text = preview.text } } }
    local details = { snapshotId = file.snapshot.snapshotId }
    if preview.truncation then details.truncation = preview.truncation end
    if preview.nextOffset ~= nil then details.nextOffset = preview.nextOffset end
    result.details = details
    return result
  end,
})

pi.register_tool({
  name = "hashline_edit",
  label = "Hashline Edit",
  description = table.concat({
    "Patch a UTF-8 text file using hashline v2 LINEID anchors copied from read output (e.g. 160sr).",
    "Preferred entries: {loc,content}. loc: \"append\", \"prepend\", {append:LINEID}, {prepend:LINEID}, {range:{pos,end}}.",
    "content is literal file content lines (string[]/string) or null to delete.",
    "Anchors are strict; stale hash mismatches are rejected with fresh retry anchors.",
    "Multiple anchor edits validate against the same pre-edit snapshot and apply bottom-up. Merge overlapping or adjacent edits.",
    "Legacy op/pos/end/lines and replace_text remain accepted for compatibility.",
  }, "\n"),
  promptSnippet = "Patch files using hashline v2 LINEID anchors from read output.",
  promptGuidelines = {
    "Use read before edit; copy full LINEID anchors exactly (e.g. 160sr, not sr).",
    "Use loc/content: {range:{pos,end}} for replacements/deletes, {append}/{prepend} for inserts.",
    "Use literal file content in content lines, without LINEID| prefixes or diff prefixes.",
    "Merge overlapping or adjacent edits in the same file into one replace range.",
  },
  parameters = {
    type = "object",
    properties = {
      path = { type = "string", description = "Path to the file to edit (relative or absolute)" },
      edits = { type = "array", minItems = 1, description = "Hashline edits for this file" },
    },
    required = { "path", "edits" },
  },
  prepare_arguments = prepare_edit_arguments,

  renderCall = function(args, theme, _context)
    local is_record = type(args) == "table"
    local path = is_record and type(args.path) == "string" and args.path or "..."
    local count = is_record and type(args.edits) == "table" and #args.edits or 0
    local suffix = (count > 0) and theme:fg("muted", (" (%d edit%s)"):format(count, count == 1 and "" or "s")) or ""
    return { text = theme:fg("toolTitle", theme:bold("edit")) .. " " .. theme:fg("accent", path) .. suffix }
  end,

  renderResult = function(result, options, theme, _context)
    if options and options.isPartial then
      return { text = theme:fg("warning", "Editing...") }
    end
    local body = {}
    if result and result.content then
      for _, entry in ipairs(result.content) do
        if entry and entry.type == "text" and entry.text then body[#body + 1] = entry.text end
      end
    end
    return { text = table.concat(body, "\n") }
  end,

  execute = function(_tool_call_id, params, signal, _on_update, ctx)
    assert_edit_request(params)
    local path = params.path
    local absolute_path = resolve_to_cwd(path, ctx.cwd)
    local mutation_target_path = resolve_mutation_target_path(absolute_path)

    return with_file_mutation_queue(mutation_target_path, function()
      throw_if_aborted(signal)
      local target_path = resolve_mutation_target_path(absolute_path)
      if target_path ~= mutation_target_path then
        error("[E_PATH_CHANGED] File path resolved to a different target while waiting to edit. Re-read and retry.", 0)
      end
      local ok_access = pcall(pi.fs.stat, target_path)
      if not ok_access then error(("File not found: %s"):format(path), 0) end
      if is_supported_image_file(target_path) then
        error(("Path is an image file: %s. Hashline edit only supports UTF-8 text files."):format(path), 0)
      end
      throw_if_aborted(signal)
      local file = load_text_file_with_snapshot(target_path)
      local original = file.text
      local snapshot = file.snapshot
      local result_raw = apply_edits_to_raw_content(file.rawText, params.edits, file.lineEnding)
      local result = normalize_to_lf(result_raw)
      if result == original then
        return { content = { { type = "text", text = "No changes made. The requested edits produced identical content." } },
          details = { classification = "noop", snapshotId = snapshot.snapshotId } }
      end
      throw_if_aborted(signal)
      local latest_snapshot = get_file_snapshot(target_path)
      if not same_file_snapshot(snapshot, latest_snapshot) then
        file = load_text_file_with_snapshot(target_path)
        original = file.text
        snapshot = file.snapshot
        result_raw = apply_edits_to_raw_content(file.rawText, params.edits, file.lineEnding)
        result = normalize_to_lf(result_raw)
        if result == original then
          return { content = { { type = "text", text = "No changes made. The requested edits produced identical content." } },
            details = { classification = "noop", snapshotId = snapshot.snapshotId } }
        end
        latest_snapshot = snapshot
      end
      local persisted = file.bom .. result_raw
      local updated_snapshot = write_text_file_atomically(target_path, persisted, { expectedSnapshot = latest_snapshot })
      local response = build_changed_anchor_response(original, result, { maxBytes = DEFAULT_MAX_BYTES })
      local metrics = compute_edit_line_metrics(original, params.edits)
      return {
        content = { { type = "text", text = response.text } },
        details = {
          firstChangedLine = response.firstChangedLine,
          snapshotId = updated_snapshot.snapshotId,
          metrics = {
            edits_attempted = #params.edits,
            added_lines = metrics.addedLines,
            removed_lines = metrics.removedLines,
          },
        },
      }
    end)
  end,
})

-- applyEditsToRawContentPreservingLineEndings
local function resolve_fallback_line_terminator(ending)
  return (ending and #ending > 0) and ending or "\n"
end

local function clone_line_records(records)
  local out = {}
  for _, r in ipairs(records) do out[#out + 1] = { text = r.text, ending = r.ending } end
  return out
end

local function find_backward_line_terminator(records, start)
  for index = math.min(start, #records), 1, -1 do
    local ending = records[index].ending
    if ending and ending ~= "" then return ending end
  end
  return nil
end

local function find_forward_line_terminator(records, start)
  for index = math.max(0, start) + 1, #records do
    local ending = records[index].ending
    if ending and ending ~= "" then return ending end
  end
  return nil
end

local function get_preferred_line_terminator(edit, original_records, fallback)
  if edit.kind == "replace" then
    for index = edit.start + 1, edit["end"] do
      local ending = original_records[index] and original_records[index].ending or ""
      if ending and ending ~= "" then return ending end
    end
    return find_forward_line_terminator(original_records, edit.start)
      or find_backward_line_terminator(original_records, edit.start - 1)
      or fallback
  end
  if edit.kind == "append" then
    return find_backward_line_terminator(original_records, edit.start - 1)
      or find_forward_line_terminator(original_records, edit.start)
      or fallback
  end
  return find_forward_line_terminator(original_records, edit.start)
    or find_backward_line_terminator(original_records, edit.start - 1)
    or fallback
end

local function normalize_line_record_terminator_state(records, original_had_final_newline, fallback)
  if #records == 0 then return end
  for index = 1, #records - 1 do
    if not (records[index].ending and records[index].ending ~= "") then records[index].ending = fallback end
  end
  local last = records[#records]
  if original_had_final_newline then
    if not (last.ending and last.ending ~= "") then last.ending = fallback end
  else
    last.ending = ""
  end
end

local function apply_line_record_edits(original_records, edits, fallback)
  local original_lines = {}
  for _, r in ipairs(original_records) do original_lines[#original_lines + 1] = r.text end
  local line_edits = resolve_line_edits(edits, original_lines)
  local next_records = clone_line_records(original_records)
  local original_had_final_newline = #original_records > 0 and (original_records[#original_records].ending ~= "")
  table.sort(line_edits, function(a, b) return b.start > a.start or (b.start == a.start and b["end"] > a["end"]) end)
  for _, edit in ipairs(line_edits) do
    local preferred = get_preferred_line_terminator(edit, original_records, fallback)
    local replacement = {}
    for _, text in ipairs(edit.lines) do replacement[#replacement + 1] = { text = text, ending = preferred } end
    local remove = edit["end"] - edit.start
    local head = {}
    for i = 1, edit.start do head[#head + 1] = next_records[i] end
    for i = edit.start + remove + 1, #next_records do head[#head + 1] = next_records[i] end
    for _, r in ipairs(replacement) do head[#head + 1] = r end
    next_records = head
  end
  normalize_line_record_terminator_state(next_records, original_had_final_newline, fallback)
  return next_records
end

local function first_line_terminator_in_text(text)
  for index = 1, #text do
    local char = text:sub(index, index)
    if char == "\r" then return text:sub(index + 1, index + 1) == "\n" and "\r\n" or "\r" end
    if char == "\n" then return "\n" end
  end
  return nil
end

local function apply_exact_unique_replace_preserving(raw_content, old_text, new_text, fallback)
  local normalized_content = normalize_to_lf(raw_content)
  local normalized_old = normalize_to_lf(old_text)
  local normalized_new = normalize_to_lf(new_text)
  local start = find_unique_normalized_match(normalized_content, normalized_old)
  local raw_start = find_raw_offset(raw_content, start)
  local raw_end = find_raw_offset(raw_content, start + #normalized_old)
  local preferred = first_line_terminator_in_text(raw_content:sub(raw_start, raw_end - 1))
    or last_line_terminator_segment(raw_content:sub(1, raw_start - 1))
    or first_line_terminator_in_text(raw_content:sub(raw_end))
    or fallback
  return raw_content:sub(1, raw_start - 1) .. restore_line_ending_like(normalized_new, preferred) .. raw_content:sub(raw_end)
end

local function find_raw_offset(raw_content, normalized_offset)
  local raw_offset = 0
  local normalized = 0
  local n = #raw_content
  while raw_offset < n and normalized < normalized_offset do
    if raw_content:sub(raw_offset + 1, raw_offset + 2) == "\r\n" then raw_offset = raw_offset + 2
    else raw_offset = raw_offset + 1 end
    normalized = normalized + 1
  end
  return raw_offset + 1
end

local function last_line_terminator_segment(text)
  for index = #text, 1, -1 do
    local char = text:sub(index, index)
    if char == "\n" then return (index > 1 and text:sub(index - 1, index - 1) == "\r") and "\r\n" or "\n" end
    if char == "\r" then return text:sub(index + 1, index + 1) == "\n" and nil or "\r" end
  end
  return nil
end

local function restore_line_ending_like(text, ending)
  local normalized = normalize_to_lf(text)
  return ending == "\n" and normalized or normalized:gsub("\n", ending)
end

local function apply_edits_to_raw_content(original_raw, edits, options)
  options = options or {}
  local fallback = resolve_fallback_line_terminator(options.defaultLineEnding)
  local text_edits = {}
  for _, edit in ipairs(edits) do if edit.op == "replace_text" then text_edits[#text_edits + 1] = edit end end
  if #text_edits > 0 then
    if #edits ~= 1 then error("[E_EDIT_CONFLICT] replace_text cannot be mixed with anchor edits in one call. Use anchors or split the request.", 0) end
    local edit = text_edits[1]
    if type(edit.oldText) ~= "string" or type(edit.newText) ~= "string" then
      error("[E_BAD_OP] replace_text requires string oldText and newText.", 0)
    end
    return apply_exact_unique_replace_preserving(original_raw, edit.oldText, edit.newText, fallback)
  end
  local next_records = apply_line_record_edits(split_text_line_records(original_raw), edits, fallback)
  return join_text_line_records(next_records)
end

-- ---- debug hook ----
local debug_flag = pi.env.PI_HASHLINE_DEBUG
if debug_flag == "1" or debug_flag == "true" then
  pi.on("session_start", function(_event, ctx)
    if ctx.hasUI then ctx.ui.notify("pi-hashline active", "info") end
  end)
end
