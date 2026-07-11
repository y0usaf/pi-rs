-- Builtin tools pack — port of the spec's `core/tools/` (pi v0.79.0),
-- authored as Lua on the public extension surface (DESIGN divergence 2).
-- The fragments in this directory are concatenated into one chunk by
-- `src/builtins/mod.rs`, one fragment per spec module, so parity audits
-- stay file-shaped. This prelude holds the chunk arguments and the JS
-- string helpers the ports share.
local pi = ...

-- The loader-injected working directory (spec: the `cwd` argument of the
-- `create*Tool` factories; the default tools receive the host cwd).
local cwd = pi.cwd()

-- JS String.prototype.split(sep): plain (non-pattern), keeps empty
-- segments including a trailing one.
local function split(s, sep)
  local out = {}
  local start = 1
  while true do
    local i = s:find(sep, start, true)
    if not i then
      out[#out + 1] = s:sub(start)
      return out
    end
    out[#out + 1] = s:sub(start, i - 1)
    start = i + #sep
  end
end

-- Format a bridge number the way JS template interpolation does:
-- integral values print without a decimal point.
local function fmt_num(n)
  local i = math.tointeger(n)
  return i and tostring(i) or tostring(n)
end

-- Node Buffer.toString("utf-8") semantics: invalid UTF-8 is replaced with
-- U+FFFD instead of raising (the bridge requires valid UTF-8 strings).
-- Divergence noted: Node emits one U+FFFD per maximal invalid subpart;
-- this replaces per invalid byte.
local function utf8_lossy(s)
  if utf8.len(s) then
    return s
  end
  local out, i, n = {}, 1, #s
  while i <= n do
    local b = s:byte(i)
    local len
    if b < 0x80 then
      len = 1
    elseif b >= 0xC2 and b <= 0xDF then
      len = 2
    elseif b >= 0xE0 and b <= 0xEF then
      len = 3
    elseif b >= 0xF0 and b <= 0xF4 then
      len = 4
    end
    local valid = len ~= nil and i + len - 1 <= n
    if valid and len > 1 then
      for k = 1, len - 1 do
        local c = s:byte(i + k)
        if c < 0x80 or c > 0xBF then
          valid = false
          break
        end
      end
      -- Reject overlongs, surrogates, and > U+10FFFF.
      if valid and not utf8.len(s:sub(i, i + len - 1)) then
        valid = false
      end
    end
    if valid then
      out[#out + 1] = s:sub(i, i + len - 1)
      i = i + len
    else
      out[#out + 1] = "\239\191\189" -- U+FFFD
      i = i + 1
    end
  end
  return table.concat(out)
end
