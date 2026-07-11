-- path-utils.ts plus the utils/paths.ts slice it calls (normalizePath /
-- resolvePath with `normalizeUnicodeSpaces` + `stripAtPrefix`, tilde
-- expansion default-on).
--
-- Divergences noted: the NFD filename variants (macOS stores filenames
-- decomposed) need a unicode-normalize binding — carried in PLAN.md; the
-- AM/PM and curly-quote variants are byte-level and ported.
local NARROW_NO_BREAK_SPACE = "\226\128\175" -- U+202F

-- utils/paths.ts UNICODE_SPACES: [\u00A0\u2000-\u200A\u202F\u205F\u3000]
-- as UTF-8 byte patterns.
local function normalize_unicode_spaces(s)
  s = s:gsub("\194\160", " ") -- U+00A0
  s = s:gsub("\226\128[\128-\138]", " ") -- U+2000–U+200A
  s = s:gsub("\226\128\175", " ") -- U+202F
  s = s:gsub("\226\129\159", " ") -- U+205F
  s = s:gsub("\227\128\128", " ") -- U+3000
  return s
end

-- utils/paths.ts normalizePath, fixed to the tools' call-site options
-- (normalizeUnicodeSpaces + stripAtPrefix; expandTilde defaults on).
local function normalize_path_input(input)
  local normalized = normalize_unicode_spaces(input)
  if normalized:sub(1, 1) == "@" then
    normalized = normalized:sub(2)
  end
  local home = pi.env.HOME
  if home and home ~= "" then
    if normalized == "~" then
      return home
    end
    if normalized:sub(1, 2) == "~/" then
      return pi.path.join(home, normalized:sub(3))
    end
  end
  if normalized:find("^file://") then
    local p = normalized:sub(8)
    p = p:gsub("%%(%x%x)", function(h)
      return string.char(tonumber(h, 16))
    end)
    return p
  end
  return normalized
end

-- Resolve a path relative to the tool cwd; handles ~ expansion and
-- absolute paths (spec resolveToCwd(path, cwd) — the base defaults to
-- the loader-injected cwd; render contexts pass their own).
local function resolve_to_cwd(file_path, base)
  return pi.path.resolve(base or cwd, normalize_path_input(file_path))
end

-- utils/paths.ts formatPathRelativeToCwdOrAbsolute: cwd-relative display
-- when the path does not escape the cwd, absolute otherwise.
local function format_path_relative_to_cwd_or_absolute(absolute_path, base)
  local relative_path = pi.path.relative(base, absolute_path)
  local inside = relative_path == ""
    or (relative_path ~= ".."
      and relative_path:sub(1, 3) ~= "../"
      and not pi.path.is_absolute(relative_path))
  if not inside then return absolute_path end
  if relative_path == "" then return "." end
  return relative_path
end

-- macOS screenshot names put a narrow no-break space before AM/PM
-- (spec: / (AM|PM)\./gi).
local function try_macos_screenshot_path(file_path)
  return (file_path:gsub(" ([AaPp][Mm])%.", NARROW_NO_BREAK_SPACE .. "%1."))
end

-- macOS uses U+2019 in screenshot names like "Capture d'écran"; users
-- type U+0027.
local function try_curly_quote_variant(file_path)
  return (file_path:gsub("'", "\226\128\153"))
end

-- Resolve a read path, trying the macOS filename variants when the plain
-- resolution does not exist (spec resolveReadPathAsync; NFD variants
-- carried).
local function resolve_read_path(file_path)
  local resolved = resolve_to_cwd(file_path)
  if pi.fs.exists(resolved) then
    return resolved
  end
  local am_pm_variant = try_macos_screenshot_path(resolved)
  if am_pm_variant ~= resolved and pi.fs.exists(am_pm_variant) then
    return am_pm_variant
  end
  local curly_variant = try_curly_quote_variant(resolved)
  if curly_variant ~= resolved and pi.fs.exists(curly_variant) then
    return curly_variant
  end
  return resolved
end
