-- Translation of Pi v0.79.0 examples/extensions/github-issue-autocomplete.ts.
-- Requires GitHub CLI (`gh`) and a GitHub repository checkout. Preloads the
-- latest open issues once per session, then filters them locally for fast
-- `#...` completion.
local pi = ...

local MAX_ISSUES = 100
local MAX_SUGGESTIONS = 20

local function extract_issue_token(text_before)
  -- (?:^|[ \t])#([^\s#]*)$
  return text_before:match("(?:^|[ \t])#([^%s#]*)$")
end

local function parse_github_repo(remote_url)
  -- git@github.com:owner/repo(.git)?
  local ssh = remote_url:match("^git@github%.com:([^/]+/[^/]+?)(%.git)?$")
  if ssh then return ssh end
  -- https?://github.com/owner/repo(.git)?
  return remote_url:match("^https?://github%.com/([^/]+/[^/]+?)(%.git)?$")
end

local function resolve_github_repo(ctx)
  local result = pi.exec("git", { "remote", "-v" }, { cwd = ctx.cwd, timeout = 5000 })
  if result.code ~= 0 then
    return nil, "github-issue-autocomplete: cwd is not a git repository"
  end
  for line in result.stdout:gmatch("[^\n]+") do
    local remote = line:match("^[^%s]+%s+([^%s]+)")
    if remote then
      local repo = parse_github_repo(remote)
      if repo then return repo end
    end
  end
  return nil, "github-issue-autocomplete: cwd is not a GitHub repository"
end

local function format_issue(issue)
  return {
    value = "#" .. issue.number,
    label = "#" .. issue.number,
    description = ("[%s] %s"):format(string.lower(issue.state or ""), issue.title or ""),
  }
end

local function filter_issues(issues, query)
  if not query or query == "" then
    local out = {}
    local n = math.min(#issues, MAX_SUGGESTIONS)
    for i = 1, n do out[i] = format_issue(issues[i]) end
    return out
  end

  -- Numeric prefix match
  if query:match("^%d+$") then
    local out = {}
    for _, issue in ipairs(issues) do
      if string.sub(tostring(issue.number), 1, #query) == query then
        out[#out + 1] = format_issue(issue)
        if #out >= MAX_SUGGESTIONS then break end
      end
    end
    if #out > 0 then return out end
  end

  -- Fuzzy filter over "number title"
  local ranked = pi.tui.fuzzy_filter(
    issues, query, function(issue) return tostring(issue.number) .. " " .. (issue.title or "") end)
  local out = {}
  for i = 1, math.min(#ranked, MAX_SUGGESTIONS) do
    out[i] = format_issue(ranked[i])
  end
  return out
end

pi.on("session_start", function(_event, ctx)
  local repo, resolve_error = resolve_github_repo(ctx)
  if not repo then
    ctx.ui.notify(resolve_error, "error")
    return
  end

  local issues_loaded = false
  local issues = {}
  local load_error_shown = false

  ctx.ui.addAutocompleteProvider(function(current)
    return {
      triggerCharacters = { "#" },
      get_suggestions = function(self, lines, cursor_line, cursor_col, _options)
        local current_line = lines[cursor_line] or ""
        local text_before = string.sub(current_line, 1, cursor_col or #current_line)
        local token = extract_issue_token(text_before)
        if not token then
          if current and current.get_suggestions then return current:get_suggestions(lines, cursor_line, cursor_col, _options) end
          return nil
        end

        -- Load issues lazily on first `#` request.
        if not issues_loaded then
          issues_loaded = true
          local gh = pi.exec("gh", {
            "issue", "list", "--repo", repo, "--state", "open", "--limit", tostring(MAX_ISSUES),
            "--json", "number,title,state",
          }, { cwd = ctx.cwd, timeout = 5000 })
          if gh.code ~= 0 then
            if not load_error_shown then
              load_error_shown = true
              local details = gh.stderr:gsub("%s+$", "")
              if details == "" then details = "exit code " .. gh.code end
              ctx.ui.notify("github-issue-autocomplete: failed to load issues: " .. details, "error")
            end
          else
            local ok, parsed = pcall(pi.json.decode, gh.stdout)
            if ok and type(parsed) == "table" then
              issues = parsed
            elseif not load_error_shown then
              load_error_shown = true
              ctx.ui.notify("github-issue-autocomplete: failed to parse gh issue list output", "error")
            end
          end
        end

        if #issues == 0 then
          if current and current.get_suggestions then return current:get_suggestions(lines, cursor_line, cursor_col, _options) end
          return nil
        end

        local suggestions = filter_issues(issues, token)
        if #suggestions == 0 then
          if current and current.get_suggestions then return current:get_suggestions(lines, cursor_line, cursor_col, _options) end
          return nil
        end

        return { items = suggestions, prefix = "#" .. token }
      end,
      apply_completion = function(self, ...)
        if current and current.apply_completion then return current:apply_completion(...) end
        return nil
      end,
      should_trigger_file_completion = function(self, ...)
        if current and current.should_trigger_file_completion then return current:should_trigger_file_completion(...) end
        return true
      end,
    }
  end)
end)