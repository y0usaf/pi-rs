-- File-backed review translation (earendil_pi-review, dogfood package).
--
-- Code Review extension: /review and /end-review commands.
-- Public surface only: events (session_start/session_tree), register_command,
-- pi.exec (git/gh), pi.json.decode, pi.fs (stat/read_file), pi.path
-- (join/dirname), ctx.cwd, ctx.sessionManager (get_branch/get_entries/
-- get_leaf_id), ctx.navigateTree, ctx.ui.* (notify/select/editor/setWidget/
-- getEditorText/setEditorText), pi.appendEntry, pi.sendUserMessage.
--
-- No long-lived host resources. Semantics preserve the sole-active-review
-- invariant: one review can be active at a time, and /end-review returns to
-- the origin branch (optionally summarizing or fixing findings first).
--
-- The upstream TypeScript declares a private TUI component tree for each
-- picker (Container/DynamicBorder/Input/SelectList/Spacer/fuzzyFilter from
-- @earendil-works/pi-tui and DynamicBorder/BorderedLoader from
-- @earendil-works/pi-coding-agent). Those are not public primitives on the
-- pi.* surface, so this translation uses the public composable-UI dialogs
-- ctx.ui.select and ctx.ui.editor as the behavioural equivalent (same choices,
-- cancel→nil, and per-mode follow-up inputs); the git/prompt/PR/state logic
-- and prompt constants are preserved verbatim.
local pi = ...

local REVIEW_STATE_TYPE = "review-session"
local REVIEW_ANCHOR_TYPE = "review-anchor"
local REVIEW_SETTINGS_TYPE = "review-settings"
local GH_SETUP_INSTRUCTIONS =
  "Install GitHub CLI (`gh`) from https://cli.github.com/ (macOS: `brew install gh`), then sign in with `gh auth login` and verify with `gh auth status`."
local PR_CHECKOUT_BLOCKED = "Cannot checkout PR: you have uncommitted changes. Please commit or stash them first."

local reviewOriginId
local endReviewInProgress = false
local reviewCustomInstructions

local function trim(s) local out = (s and (s:gsub("^%s*(.-)%s*$", "%1"))) or "" return out end

-- getReviewState / getReviewSettings read the persisted review state from the
-- branch / full entry list exactly like the source (custom-type entries).
local function getReviewState(ctx)
  local state
  for _, entry in ipairs(ctx.sessionManager.get_branch()) do
    if entry and entry.type == "custom" and entry.customType == REVIEW_STATE_TYPE then state = entry.data end
  end
  return state
end

local function setReviewWidget(ctx, active)
  if not ctx.hasUI then return end
  if not active then
    ctx.ui.setWidget("review", nil)
    return
  end
  ctx.ui.setWidget("review", function(theme)
    return { text = theme:fg("warning", "Review session active, return with /end-review") }
  end, {})
end

local function applyReviewState(ctx)
  local state = getReviewState(ctx)
  if state and state.active and state.originId then
    reviewOriginId = state.originId
    setReviewWidget(ctx, true)
    return
  end
  reviewOriginId = nil
  setReviewWidget(ctx, false)
end

local function getReviewSettings(ctx)
  local state
  for _, entry in ipairs(ctx.sessionManager.get_entries()) do
    if entry and entry.type == "custom" and entry.customType == REVIEW_SETTINGS_TYPE then state = entry.data end
  end
  return { customInstructions = state and trim(state.customInstructions) or nil }
end

local function applyReviewSettings(ctx)
  reviewCustomInstructions = getReviewSettings(ctx).customInstructions
end

local function applyAllReviewState(ctx)
  applyReviewSettings(ctx)
  applyReviewState(ctx)
end

local function persistReviewSettings()
  pi.appendEntry(REVIEW_SETTINGS_TYPE, { customInstructions = reviewCustomInstructions })
end

local function setReviewCustomInstructions(instructions)
  reviewCustomInstructions = trim(instructions) or nil
  persistReviewSettings()
end

-- REVIEW_GUIDELINES.md discovery: walk up from cwd looking for a `.pi`
-- directory (same dir as the project instructions). If one exists and a
-- REVIEW_GUIDELINES.md file sits beside it, return its trimmed contents.
local function loadProjectReviewGuidelines(cwd)
  local currentDir = pi.path.resolve(cwd)
  while true do
    local piDir = pi.path.join(currentDir, ".pi")
    local guidelinesPath = pi.path.join(currentDir, "REVIEW_GUIDELINES.md")
    local ok, stat = pcall(pi.fs.stat, piDir)
    if ok and stat and stat.type == "dir" then
      local gOk, gStat = pcall(pi.fs.stat, guidelinesPath)
      if gOk and gStat and gStat.type == "file" then
        local rOk, content = pcall(pi.fs.read_file, guidelinesPath)
        if rOk and content then
          local trimmedContent = trim(content)
          return trimmedContent ~= "" and trimmedContent or nil
        end
      end
      return nil
    end
    local parentDir = pi.path.dirname(currentDir)
    if parentDir == currentDir then return nil end
    currentDir = parentDir
  end
end

local function getMergeBase(branch)
  local function run(args)
    local ok, res = pcall(pi.exec, "git", args)
    if not ok or (res and res.code ~= 0) then return nil end
    local out = trim(res.stdout)
    if out == "" then return nil end
    return out
  end
  local upstream = run({ "rev-parse", "--abbrev-ref", branch .. "@{upstream}" })
  if upstream then
    local mb = run({ "merge-base", "HEAD", upstream })
    if mb then return mb end
  end
  return run({ "merge-base", "HEAD", branch })
end

local function getLocalBranches()
  local ok, res = pcall(pi.exec, "git", { "branch", "--format=%(refname:short)" })
  if not ok or (res and res.code ~= 0) then return {} end
  local out = {}
  for line in (trim(res.stdout) .. "\n"):gmatch("(.-)\n") do
    if trim(line) ~= "" then out[#out + 1] = trim(line) end
  end
  return out
end

local function getRecentCommits(limit)
  limit = limit or 10
  local ok, res = pcall(pi.exec, "git", { "log", "--oneline", "-n", tostring(limit) })
  if not ok or (res and res.code ~= 0) then return {} end
  local out = {}
  for line in (trim(res.stdout) .. "\n"):gmatch("(.-)\n") do
    local l = trim(line)
    if l ~= "" then
      local sha, rest = l:match("^(%S+)%s*(.-)$")
      out[#out + 1] = { sha = sha, title = trim(rest or "") }
    end
  end
  return out
end

local function hasUncommittedChanges()
  local ok, res = pcall(pi.exec, "git", { "status", "--porcelain" })
  return ok and (res and res.code == 0) and trim(res.stdout) ~= ""
end

local function hasPendingChanges()
  local ok, res = pcall(pi.exec, "git", { "status", "--porcelain" })
  if not ok or (res and res.code ~= 0) then return false end
  local count = 0
  for line in (trim(res.stdout) .. "\n"):gmatch("(.-)\n") do
    local l = trim(line)
    if l ~= "" and l:sub(1, 2) ~= "??" then count = count + 1 end
  end
  return count > 0
end

local function getCurrentBranch()
  local ok, res = pcall(pi.exec, "git", { "branch", "--show-current" })
  if ok and (res and res.code == 0) and trim(res.stdout) ~= "" then return trim(res.stdout) end
  return nil
end

local function getDefaultBranch()
  local ok, res = pcall(pi.exec, "git", { "symbolic-ref", "refs/remotes/origin/HEAD", "--short" })
  if ok and (res and res.code == 0) and trim(res.stdout) ~= "" then
    return trim(res.stdout):gsub("^origin/", "")
  end
  local branches = getLocalBranches()
  for _, b in ipairs(branches) do if b == "main" then return "main" end end
  for _, b in ipairs(branches) do if b == "master" then return "master" end end
  return "main"
end

local function parsePrReference(ref)
  local trimmed = trim(ref)
  local num = tonumber(trimmed)
  if num and num > 0 then return num end
  local m = trimmed:match("github%.com/[^/]+/[^/]+/pull/(%d+)")
  if m then return tonumber(m) end
  return nil
end

local function getPrInfo(prNumber)
  local ok, res = pcall(pi.exec, "gh", { "pr", "view", tostring(prNumber), "--json", "baseRefName,title,headRefName" })
  if not ok or (res and res.code ~= 0) then return nil end
  local p_ok, data = pcall(pi.json.decode, res.stdout)
  if not p_ok or type(data) ~= "table" then return nil end
  return { baseBranch = data.baseRefName, title = data.title, headBranch = data.headRefName }
end

local function checkoutPr(prNumber)
  local ok, res = pcall(pi.exec, "gh", { "pr", "checkout", tostring(prNumber) })
  if not ok or (res and res.code ~= 0) then
    local msg = (ok and (res and (res.stderr or res.stdout))) or tostring(res)
    return { success = false, error = (msg ~= "" and msg or "Failed to checkout PR") }
  end
  return { success = true }
end

-- Review prompts + the detailed rubric (preserved verbatim from review.ts).
local UNCOMMITTED_PROMPT = "Review the current code changes (staged, unstaged, and untracked files) and provide prioritized findings."
local BASE_BRANCH_PROMPT_WITH_MERGE_BASE = "Review the code changes against the base branch '{baseBranch}'. The merge base commit for this comparison is {mergeBaseSha}. Run `git diff {mergeBaseSha}` to inspect the changes relative to {baseBranch}. Provide prioritized, actionable findings."
local BASE_BRANCH_PROMPT_FALLBACK = "Review the code changes against the base branch '{branch}'. Start by finding the merge diff between the current branch and {branch}'s upstream e.g. (`git merge-base HEAD \"$(git rev-parse --abbrev-ref \"{branch}@{upstream}\")\"`), then run `git diff` against that SHA to see what changes we would merge into the {branch} branch. Provide prioritized, actionable findings."
local COMMIT_PROMPT_WITH_TITLE = 'Review the code changes introduced by commit {sha} ("{title}"). Provide prioritized, actionable findings.'
local COMMIT_PROMPT = "Review the code changes introduced by commit {sha}. Provide prioritized, actionable findings."
local PULL_REQUEST_PROMPT = 'Review pull request #{prNumber} ("{title}") against the base branch \'{baseBranch}\'. The merge base commit for this comparison is {mergeBaseSha}. Run `git diff {mergeBaseSha}` to inspect the changes that would be merged. Provide prioritized, actionable findings.'
local PULL_REQUEST_PROMPT_FALLBACK = 'Review pull request #{prNumber} ("{title}") against the base branch \'{baseBranch}\'. Start by finding the merge base between the current branch and {baseBranch} (e.g., `git merge-base HEAD {baseBranch}`), then run `git diff` against that SHA to see the changes that would be merged. Provide prioritized, actionable findings.'
local FOLDER_REVIEW_PROMPT = "Review the code in the following paths: {paths}. This is a snapshot review (not a diff). Read the files directly in these paths and provide prioritized, actionable findings."

local REVIEW_RUBRIC = [[# Review Guidelines

You are acting as a code reviewer for a proposed code change made by another engineer.

Below are default guidelines for determining what to flag. These are not the final word — if you encounter more specific guidelines elsewhere (in a developer message, user message, file, or project review guidelines appended below), those override these general instructions.

## Determining what to flag

Flag issues that:
1. Meaningfully impact the accuracy, performance, security, or maintainability of the code.
2. Are discrete and actionable (not general issues or multiple combined issues).
3. Don't demand rigor inconsistent with the rest of the codebase.
4. Were introduced in the changes being reviewed (not pre-existing bugs).
5. The author would likely fix if aware of them.
6. Don't rely on unstated assumptions about the codebase or author's intent.
7. Have provable impact on other parts of the code — it is not enough to speculate that a change may disrupt another part, you must identify the parts that are provably affected.
8. Are clearly not intentional changes by the author.
9. Be particularly careful with untrusted user input and follow the specific guidelines to review.
10. Treat silent local error recovery (especially parsing/IO/network fallbacks) as high-signal review candidates unless there is explicit boundary-level justification.

## Untrusted User Input

1. Be careful with open redirects, they must always be checked to only go to trusted domains (?next_page=...)
2. Always flag SQL that is not parametrized
3. In systems with user supplied URL input, http fetches always need to be protected against access to local resources (intercept DNS resolver!)
4. Escape, don't sanitize if you have the option (eg: HTML escaping)

## Comment guidelines

1. Be clear about why the issue is a problem.
2. Communicate severity appropriately - don't exaggerate.
3. Be brief - at most 1 paragraph.
4. Keep code snippets under 3 lines, wrapped in inline code or code blocks.
5. Use ```suggestion blocks ONLY for concrete replacement code (minimal lines; no commentary inside the block). Preserve the exact leading whitespace of the replaced lines.
6. Explicitly state scenarios/environments where the issue arises.
7. Use a matter-of-fact tone - helpful AI assistant, not accusatory.
8. Write for quick comprehension without close reading.
9. Avoid excessive flattery or unhelpful phrases like "Great job...".

## Review priorities

1. Surface critical non-blocking human callouts (migrations, dependency churn, auth/permissions, compatibility, destructive operations) at the end.
2. Prefer simple, direct solutions over wrappers or abstractions without clear value.
3. Treat back pressure handling as critical to system stability.
4. Apply system-level thinking; flag changes that increase operational risk or on-call wakeups.
5. Ensure that errors are always checked against codes or stable identifiers, never error messages.

## Fail-fast error handling (strict)

When reviewing added or modified error handling, default to fail-fast behavior.

1. Evaluate every new or changed try/catch: identify what can fail and why local handling is correct at that exact layer.
2. Prefer propagation over local recovery. If the current scope cannot fully recover while preserving correctness, rethrow (optionally with context) instead of returning fallbacks.
3. Flag catch blocks that hide failure signals (e.g. returning null/[]/false, swallowing JSON parse failures, logging-and-continue, or "best effort" silent recovery).
4. JSON parsing/decoding should fail loudly by default. Quiet fallback parsing is only acceptable with an explicit compatibility requirement and clear tested behavior.
5. Boundary handlers (HTTP routes, CLI entrypoints, supervisors) may translate errors, but must not pretend success or silently degrade.
6. If a catch exists only to satisfy lint/style without real handling, treat it as a bug.
7. When uncertain, prefer crashing fast over silent degradation.

## Required human callouts (non-blocking, at the very end)

After findings/verdict, you MUST append this final section:

## Human Reviewer Callouts (Non-Blocking)

Include only applicable callouts (no yes/no lines):

- **This change adds a database migration:** <files/details>
- **This change introduces a new dependency:** <package(s)/details>
- **This change changes a dependency (or the lockfile):** <files/package(s)/details>
- **This change modifies auth/permission behavior:** <what changed and where>
- **This change introduces backwards-incompatible public schema/API/contract changes:** <what changed and where>
- **This change includes irreversible or destructive operations:** <operation and scope>
- **This change adds or removes feature flags:** <feature flags changed> (call out re-use of dormant feature flags!)
- **This change changes configuration defaults:** <config var changed>

Rules for this section:
1. These are informational callouts for the human reviewer, not fix items.
2. Do not include them in Findings unless there is an independent defect.
3. These callouts alone must not change the verdict.
4. Only include callouts that apply to the reviewed change.
5. Keep each emitted callout bold exactly as written.
6. If none apply, write "- (none)".

## Priority levels

Tag each finding with a priority level in the title:
- [P0] - Drop everything to fix. Blocking release/operations. Only for universal issues that do not depend on assumptions about inputs.
- [P1] - Urgent. Should be addressed in the next cycle.
- [P2] - Normal. To be fixed eventually.
- [P3] - Low. Nice to have.

## Output format

Provide your findings in a clear, structured format:
1. List each finding with its priority tag, file location, and explanation.
2. Findings must reference locations that overlap with the actual diff — don't flag pre-existing code.
3. Keep line references as short as possible (avoid ranges over 5-10 lines; pick the most suitable subrange).
4. Provide an overall verdict: "correct" (no blocking issues) or "needs attention" (has blocking issues).
5. Ignore trivial style issues unless they obscure meaning or violate documented standards.
6. Do not generate a full PR fix — only flag issues and optionally provide short suggestion blocks.
7. End with the required "Human Reviewer Callouts (Non-Blocking)" section and all applicable bold callouts (no yes/no).

Output all findings the author would fix if they knew about them. If there are no qualifying findings, explicitly state the code looks good. Don't stop at the first finding - list every qualifying issue. Then append the required non-blocking callouts section.]]

local function renderPrompt(template, subs)
  for k, v in pairs(subs) do template = template:gsub("{" .. k .. "}", tostring(v)) end
  return template
end

local function buildReviewPrompt(target)
  if target.type == "uncommitted" then return UNCOMMITTED_PROMPT end
  if target.type == "baseBranch" then
    local mergeBase = getMergeBase(target.branch)
    return mergeBase and renderPrompt(BASE_BRANCH_PROMPT_WITH_MERGE_BASE, { baseBranch = target.branch, mergeBaseSha = mergeBase })
      or renderPrompt(BASE_BRANCH_PROMPT_FALLBACK, { branch = target.branch })
  end
  if target.type == "commit" then
    if trim(target.title) ~= "" then
      return renderPrompt(COMMIT_PROMPT_WITH_TITLE, { sha = target.sha, title = target.title })
    end
    return renderPrompt(COMMIT_PROMPT, { sha = target.sha })
  end
  if target.type == "pullRequest" then
    local mergeBase = getMergeBase(target.baseBranch)
    if mergeBase then
      return renderPrompt(PULL_REQUEST_PROMPT, { prNumber = target.prNumber, title = target.title, baseBranch = target.baseBranch, mergeBaseSha = mergeBase })
    end
    return renderPrompt(PULL_REQUEST_PROMPT_FALLBACK, { prNumber = target.prNumber, title = target.title, baseBranch = target.baseBranch })
  end
  if target.type == "folder" then
    return renderPrompt(FOLDER_REVIEW_PROMPT, { paths = table.concat(target.paths, ", ") })
  end
  return ""
end

local function getUserFacingHint(target)
  if target.type == "uncommitted" then return "current changes" end
  if target.type == "baseBranch" then return "changes against '" .. target.branch .. "'" end
  if target.type == "commit" then
    local short = target.sha:sub(1, 7)
    if target.title and trim(target.title) ~= "" then return ("commit %s: %s"):format(short, target.title) end
    return ("commit %s"):format(short)
  end
  if target.type == "pullRequest" then
    local shortTitle = #target.title > 30 and (target.title:sub(1, 27) .. "...") or target.title
    return ("PR #%d: %s"):format(target.prNumber, shortTitle)
  end
  if target.type == "folder" then
    local joined = table.concat(target.paths, ", ")
    return #joined > 40 and ("folders: %s..."):format(joined:sub(1, 37)) or ("folders: %s"):format(joined)
  end
  return ""
end

local PRESETS = {
  { value = "uncommitted", label = "Review uncommitted changes", description = "" },
  { value = "baseBranch", label = "Review against a base branch", description = "(local)" },
  { value = "commit", label = "Review a commit", description = "" },
  { value = "pullRequest", label = "Review a pull request", description = "(GitHub PR)" },
  { value = "folder", label = "Review a folder (or more)", description = "(snapshot, not diff)" },
}
local TOGGLE_CUSTOM = "toggleCustomInstructions"

local function ensureGithubCliReady(ctx)
  local ghVersion = pi.exec("gh", { "--version" })
  if ghVersion.code ~= 0 then
    ctx.ui.notify(("PR review requires GitHub CLI (`gh`). %s"):format(GH_SETUP_INSTRUCTIONS), "error")
    return false
  end
  local ghAuth = pi.exec("gh", { "auth", "status" })
  if ghAuth.code ~= 0 then
    ctx.ui.notify("GitHub CLI is installed, but you're not signed in. Run `gh auth login`, then verify with `gh auth status`.", "error")
    return false
  end
  return true
end

local function resolvePullRequestTarget(ctx, ref, options)
  options = options or {}
  if not ensureGithubCliReady(ctx) then return nil end
  if not options.skipInitialPendingChangesCheck and hasPendingChanges() then
    ctx.ui.notify(PR_CHECKOUT_BLOCKED, "error")
    return nil
  end
  local prNumber = parsePrReference(ref)
  if not prNumber then
    ctx.ui.notify("Invalid PR reference. Enter a number or GitHub PR URL.", "error")
    return nil
  end
  ctx.ui.notify(("Fetching PR #%d info..."):format(prNumber), "info")
  local prInfo = getPrInfo(prNumber)
  if not prInfo then
    ctx.ui.notify(("Could not fetch PR #%d. Make sure it exists and your GitHub auth has access (check with `gh auth status`)."):format(prNumber), "error")
    return nil
  end
  if hasPendingChanges() then
    ctx.ui.notify(PR_CHECKOUT_BLOCKED, "error")
    return nil
  end
  ctx.ui.notify(("Checking out PR #%d..."):format(prNumber), "info")
  local checkoutResult = checkoutPr(prNumber)
  if not checkoutResult.success then
    ctx.ui.notify(("Failed to checkout PR: %s"):format(checkoutResult.error), "error")
    return nil
  end
  ctx.ui.notify(("Checked out PR #%d (%s)"):format(prNumber, prInfo.headBranch), "info")
  return { type = "pullRequest", prNumber = prNumber, baseBranch = prInfo.baseBranch, title = prInfo.title }
end

local function getSmartDefault()
  if hasUncommittedChanges() then return "uncommitted" end
  local current = getCurrentBranch()
  local def = getDefaultBranch()
  if current and current ~= def then return "baseBranch" end
  return "commit"
end

local function executeReview(ctx, target, useFreshSession, options)
  options = options or {}
  if reviewOriginId then
    ctx.ui.notify("Already in a review. Use /end-review to finish first.", "warning")
    return false
  end
  if useFreshSession then
    local originId = ctx.sessionManager.get_leaf_id()
    if not originId then
      pi.appendEntry(REVIEW_ANCHOR_TYPE, { createdAt = os.date("!%Y-%m-%dT%H:%M:%SZ") })
      originId = ctx.sessionManager.get_leaf_id()
    end
    if not originId then
      ctx.ui.notify("Failed to determine review origin.", "error")
      return false
    end
    reviewOriginId = originId
    local lockedOriginId = originId
    local entries = ctx.sessionManager.get_entries()
    local firstUserMessage
    for _, e in ipairs(entries) do
      if e and e.type == "message" and e.message and e.message.role == "user" then firstUserMessage = e break end
    end
    if firstUserMessage then
      local ok, result = pcall(ctx.navigateTree, firstUserMessage.id, { summarize = false, label = "code-review" })
      if not ok then
        reviewOriginId = nil
        ctx.ui.notify(("Failed to start review: %s"):format(tostring(result)), "error")
        return false
      end
      if result and result.cancelled then
        reviewOriginId = nil
        return false
      end
      ctx.ui.setEditorText("")
    end
    reviewOriginId = lockedOriginId
    setReviewWidget(ctx, true)
    pi.appendEntry(REVIEW_STATE_TYPE, { active = true, originId = lockedOriginId })
  end
  local prompt = buildReviewPrompt(target)
  local hint = getUserFacingHint(target)
  local projectGuidelines = loadProjectReviewGuidelines(ctx.cwd)

  local fullPrompt = REVIEW_RUBRIC .. "\n\n---\n\nPlease perform a code review with the following focus:\n\n" .. prompt
  if reviewCustomInstructions then fullPrompt = fullPrompt .. "\n\nShared custom review instructions (applies to all reviews):\n\n" .. reviewCustomInstructions end
  if options.extraInstruction and trim(options.extraInstruction) ~= "" then
    fullPrompt = fullPrompt .. "\n\nAdditional user-provided review instruction:\n\n" .. trim(options.extraInstruction)
  end
  if projectGuidelines then
    fullPrompt = fullPrompt .. "\n\nThis project has additional instructions for code reviews:\n\n" .. projectGuidelines
  end
  local modeHint = useFreshSession and " (fresh session)" or ""
  ctx.ui.notify(("Starting review: %s%s"):format(hint, modeHint), "info")
  pi.sendUserMessage(fullPrompt)
  return true
end

local function tokenizeArgs(value)
  local tokens = {}
  local current = ""
  local quote
  local i = 1
  while i <= #value do
    local char = value:sub(i, i)
    if quote then
      if char == "\\" and i + 1 <= #value then
        current = current .. value:sub(i + 1, i + 1)
        i = i + 1
      elseif char == quote then
        quote = nil
      else
        current = current .. char
      end
    elseif char == '"' or char == "'" then
      quote = char
    elseif char:match("%s") then
      if #current > 0 then tokens[#tokens + 1] = current current = "" end
    else
      current = current .. char
    end
    i = i + 1
  end
  if #current > 0 then tokens[#tokens + 1] = current end
  return tokens
end

local function parseReviewPaths(value)
  local out = {}
  for item in (value .. " "):gmatch("(%S+)") do if trim(item) ~= "" then out[#out + 1] = item end end
  return out
end

local function parseArgs(args)
  if not args or trim(args) == "" then return { target = nil } end
  local rawParts = tokenizeArgs(trim(args))
  local parts = {}
  local extraInstruction
  local i = 1
  while i <= #rawParts do
    local part = rawParts[i]
    if part == "--extra" then
      local nextv = rawParts[i + 1]
      if not nextv then return { target = nil, error = "Missing value for --extra" } end
      extraInstruction = nextv
      i = i + 1
    elseif part:sub(1, 8) == "--extra=" then
      extraInstruction = part:sub(9)
    else
      parts[#parts + 1] = part
    end
    i = i + 1
  end
  if #parts == 0 then return { target = nil, extraInstruction = extraInstruction } end
  local subcommand = parts[1]:lower()
  if subcommand == "uncommitted" then return { target = { type = "uncommitted" }, extraInstruction = extraInstruction } end
  if subcommand == "branch" then
    local branch = parts[2]
    if not branch then return { target = nil, extraInstruction = extraInstruction } end
    return { target = { type = "baseBranch", branch = branch }, extraInstruction = extraInstruction }
  end
  if subcommand == "commit" then
    local sha = parts[2]
    if not sha then return { target = nil, extraInstruction = extraInstruction } end
    local title
    if #parts > 2 then
      local t = {}
      for j = 3, #parts do t[#t + 1] = parts[j] end
      title = table.concat(t, " ")
    end
    return { target = { type = "commit", sha = sha, title = title }, extraInstruction = extraInstruction }
  end
  if subcommand == "folder" then
    local joined = {}
    for j = 2, #parts do joined[#joined + 1] = parts[j] end
    local paths = parseReviewPaths(table.concat(joined, " "))
    if #paths == 0 then return { target = nil, extraInstruction = extraInstruction } end
    return { target = { type = "folder", paths = paths }, extraInstruction = extraInstruction }
  end
  if subcommand == "pr" then
    local ref = parts[2]
    if not ref then return { target = nil, extraInstruction = extraInstruction } end
    return { target = { type = "pr", ref = ref }, extraInstruction = extraInstruction }
  end
  return { target = nil, extraInstruction = extraInstruction }
end

local function findLabelIndex(items, label)
  for i, p in ipairs(items) do if p.label == label then return i end end
  return 1
end

-- The upstream builds a private picker component tree for the preset selector.
-- Public equivalent: ctx.ui.select over the same preset labels, re-browse by
-- flipping "Add/Remove custom review instructions" or cancelling.
local function showReviewSelector(ctx)
  local smartDefault = getSmartDefault()
  local presetItems = {}
  for _, p in ipairs(PRESETS) do presetItems[#presetItems + 1] = p end
  while true do
    local items = {}
    for _, p in ipairs(presetItems) do items[#items + 1] = p end
    local toggleLabel = reviewCustomInstructions and "Remove custom review instructions" or "Add custom review instructions"
    local toggleDesc = reviewCustomInstructions and "(currently set)" or "(applies to all review modes)"
    items[#items + 1] = { value = TOGGLE_CUSTOM, label = toggleLabel, description = toggleDesc }

    local optionValues = {}
    for _, p in ipairs(items) do optionValues[#optionValues + 1] = p.label end
    local chosen = ctx.ui.select("Select a review preset", optionValues)
    if chosen == nil then return nil end
    local selected = items[findLabelIndex(items, chosen)]
    local result = selected and selected.value or nil
    if not result then return nil end

    if result == TOGGLE_CUSTOM then
      if reviewCustomInstructions then
        setReviewCustomInstructions(nil)
        ctx.ui.notify("Custom review instructions removed", "info")
      else
        local customInstructions = ctx.ui.editor("Enter custom review instructions (applies to all review modes):", "")
        if not customInstructions or trim(customInstructions) == "" then
          ctx.ui.notify("Custom review instructions not changed", "info")
        else
          setReviewCustomInstructions(customInstructions)
          ctx.ui.notify("Custom review instructions saved", "info")
        end
      end
    elseif result == "uncommitted" then
      return { type = "uncommitted" }
    elseif result == "baseBranch" then
      local branches = getLocalBranches()
      local current = getCurrentBranch()
      local def = getDefaultBranch()
      local candidates = {}
      for _, b in ipairs(branches) do if not current or b ~= current then candidates[#candidates + 1] = b end end
      if #candidates == 0 then
        ctx.ui.notify(current and ("No other branches found (current branch: %s)"):format(current) or "No branches found", "error")
        return nil
      end
      table.sort(candidates, function(a, b)
        if a == def then return true end
        if b == def then return false end
        return a < b
      end)
      local labels = {}
      for _, b in ipairs(candidates) do labels[#labels + 1] = b .. (b == def and " (default)" or "") end
      local branch = ctx.ui.select("Select base branch", labels)
      if not branch then return nil end
      for _, b in ipairs(candidates) do
        if branch == b .. (b == def and " (default)" or "") then return { type = "baseBranch", branch = b } end
      end
      return { type = "baseBranch", branch = branch }
    elseif result == "commit" then
      local commits = getRecentCommits(20)
      if #commits == 0 then ctx.ui.notify("No commits found", "error") return nil end
      local labels = {}
      for _, c in ipairs(commits) do labels[#labels + 1] = c.sha:sub(1, 7) .. " " .. c.title end
      local chosenLabel = ctx.ui.select("Select commit to review", labels)
      if not chosenLabel then return nil end
      for _, c in ipairs(commits) do
        if c.sha:sub(1, 7) .. " " .. c.title == chosenLabel then return { type = "commit", sha = c.sha, title = c.title } end
      end
      return nil
    elseif result == "folder" then
      local str = ctx.ui.editor("Enter folders/files to review (space-separated or one per line):", ".")
      if not str or trim(str) == "" then return nil end
      local paths = parseReviewPaths(str)
      if #paths == 0 then return nil end
      return { type = "folder", paths = paths }
    elseif result == "pullRequest" then
      if hasPendingChanges() then
        ctx.ui.notify(PR_CHECKOUT_BLOCKED, "error")
        return nil
      end
      local ref = ctx.ui.editor("Enter PR number or URL (e.g. 123 or https://github.com/owner/repo/pull/123):", "")
      if not ref or trim(ref) == "" then return nil end
      return resolvePullRequestTarget(ctx, ref, { skipInitialPendingChangesCheck = true })
    else
      return nil
    end
  end
end

pi.on("session_start", function(_event, ctx) applyAllReviewState(ctx) end)
pi.on("session_tree", function(_event, ctx) applyAllReviewState(ctx) end)

local REVIEW_SUMMARY_PROMPT = [[We are leaving a code-review branch and returning to the main coding branch.
Create a structured handoff that can be used immediately to implement fixes.

You MUST summarize the review that happened in this branch so findings can be acted on.
Do not omit findings: include every actionable issue that was identified.

Required sections (in order):

## Review Scope
- What was reviewed (files/paths, changes, and scope)

## Verdict
- "correct" or "needs attention"

## Findings
For EACH finding, include:
- Priority tag ([P0]..[P3]) and short title
- File location (`path/to/file.ext:line`)
- Why it matters (brief)
- What should change (brief, actionable)

## Fix Queue
1. Ordered implementation checklist (highest priority first)

## Constraints & Preferences
- Any constraints or preferences mentioned during review
- Or "(none)"

## Human Reviewer Callouts (Non-Blocking)
Include only applicable callouts (no yes/no lines):
- **This change adds a database migration:** <files/details>
- **This change introduces a new dependency:** <package(s)/details>
- **This change changes a dependency (or the lockfile):** <files/package(s)/details>
- **This change modifies auth/permission behavior:** <what changed and where>
- **This change introduces backwards-incompatible public schema/API/contract changes:** <what changed and where>
- **This change includes irreversible or destructive operations:** <operation and scope>

If none apply, write "- (none)".

These are informational callouts for humans and are not fix items by themselves.

Preserve exact file paths, function names, and error messages where available.]]

local REVIEW_FIX_FINDINGS_PROMPT = [[Use the latest review summary in this session and implement the review findings now.

Instructions:
1. Treat the summary's Findings/Fix Queue as a checklist.
2. Fix in priority order: P0, P1, then P2 (include P3 if quick and safe).
3. If a finding is invalid/already fixed/not possible right now, briefly explain why and continue.
4. Treat "Human Reviewer Callouts (Non-Blocking)" as informational only; do not convert them into fix tasks unless there is a separate explicit finding.
5. Follow fail-fast error handling: do not add local catch/fallback recovery unless this scope is an explicit boundary that can safely translate the failure.
6. If you add or keep a try/catch, explain the expected failure mode and either rethrow with context or return a boundary-safe error response.
7. JSON parsing/decoding should fail loudly by default; avoid silent fallback parsing.
8. Run relevant tests/checks for touched code where practical.
9. End with: fixed items, deferred/skipped items (with reasons), and verification results.]]

local function clearReviewState(ctx)
  setReviewWidget(ctx, false)
  reviewOriginId = nil
  pi.appendEntry(REVIEW_STATE_TYPE, { active = false })
end

local function getActiveReviewOrigin(ctx)
  if reviewOriginId then return reviewOriginId end
  local state = getReviewState(ctx)
  if state and state.active and state.originId then
    reviewOriginId = state.originId
    return reviewOriginId
  end
  if state and state.active then
    setReviewWidget(ctx, false)
    pi.appendEntry(REVIEW_STATE_TYPE, { active = false })
    ctx.ui.notify("Review state was missing origin info; cleared review status.", "warning")
  end
  return nil
end

local function navigateWithSummary(ctx, originId, showLoader)
  -- The upstream wraps navigation in a private BorderedLoader during the async
  -- summarization. Public equivalent: no custom-loader overlay; navigate with
  -- the summary instructions directly (same summarize/customInstructions/
  -- replaceInstructions contract). showLoader is accepted for source parity but
  -- the loader is a custom-UI component, so it is not replicated.
  local ok, res = pcall(ctx.navigateTree, originId, {
    summarize = true, customInstructions = REVIEW_SUMMARY_PROMPT, replaceInstructions = true,
  })
  if not ok then return { cancelled = false, error = tostring(res) } end
  return res
end

local function executeEndReviewAction(ctx, action, options)
  options = options or {}
  local originId = getActiveReviewOrigin(ctx)
  if not originId then
    local state = getReviewState(ctx)
    if not (state and state.active) then
      ctx.ui.notify("Not in a review branch (use /review first, or review was started in current session mode)", "info")
    end
    return "error"
  end
  local notifySuccess = options.notifySuccess ~= false
  if action == "returnOnly" then
    local ok, result = pcall(ctx.navigateTree, originId, { summarize = false })
    if not ok then ctx.ui.notify(("Failed to return: %s"):format(tostring(result)), "error") return "error" end
    if result and result.cancelled then
      ctx.ui.notify("Navigation cancelled. Use /end-review to try again.", "info")
      return "cancelled"
    end
    clearReviewState(ctx)
    if notifySuccess then ctx.ui.notify("Review complete! Returned to original position.", "info") end
    return "ok"
  end
  local summaryResult = navigateWithSummary(ctx, originId, options.showSummaryLoader or false)
  if summaryResult == nil then
    ctx.ui.notify("Summarization cancelled. Use /end-review to try again.", "info")
    return "cancelled"
  end
  if summaryResult.error then
    ctx.ui.notify(("Summarization failed: %s"):format(summaryResult.error), "error")
    return "error"
  end
  if summaryResult.cancelled then
    ctx.ui.notify("Navigation cancelled. Use /end-review to try again.", "info")
    return "cancelled"
  end
  clearReviewState(ctx)
  if action == "returnAndSummarize" then
    if trim(ctx.ui.getEditorText()) == "" then ctx.ui.setEditorText("Act on the review findings") end
    if notifySuccess then ctx.ui.notify("Review complete! Returned and summarized.", "info") end
    return "ok"
  end
  pi.sendUserMessage(REVIEW_FIX_FINDINGS_PROMPT, { deliverAs = "followUp" })
  if notifySuccess then ctx.ui.notify("Review complete! Returned and queued a follow-up to fix findings.", "info") end
  return "ok"
end

local function runEndReview(ctx)
  if not ctx.hasUI then
    ctx.ui.notify("End-review requires interactive mode", "error")
    return
  end
  if endReviewInProgress then ctx.ui.notify("/end-review is already running", "info") return end
  endReviewInProgress = true
  pcall(function()
    local choice = ctx.ui.select("Finish review:", { "Return only", "Return and fix findings", "Return and summarize" })
    if choice == nil then
      ctx.ui.notify("Cancelled. Use /end-review to try again.", "info")
      return
    end
    local action = (choice == "Return and fix findings" and "returnAndFix")
      or (choice == "Return and summarize" and "returnAndSummarize")
      or "returnOnly"
    executeEndReviewAction(ctx, action, { showSummaryLoader = true, notifySuccess = true })
  end)
  endReviewInProgress = false
end

pi.register_command("review", {
  description = "Review code changes (PR, uncommitted, branch, commit, or folder)",
  handler = function(args, ctx)
    if not ctx.hasUI then
      ctx.ui.notify("Review requires interactive mode", "error")
      return
    end
    if reviewOriginId then
      ctx.ui.notify("Already in a review. Use /end-review to finish first.", "warning")
      return
    end
    local gitCheck = pi.exec("git", { "rev-parse", "--git-dir" })
    if gitCheck.code ~= 0 then
      ctx.ui.notify("Not a git repository", "error")
      return
    end
    local parsed = parseArgs(args)
    if parsed.error then ctx.ui.notify(parsed.error, "error") return end
    local target = parsed.target
    local fromSelector = not target
    local extraInstruction = parsed.extraInstruction and trim(parsed.extraInstruction) or nil
    if target and target.type == "pr" then
      target = resolvePullRequestTarget(ctx, target.ref)
      if not target then ctx.ui.notify("PR review failed. Returning to review menu.", "warning") end
    end
    while true do
      if not target and fromSelector then target = showReviewSelector(ctx) end
      if not target then
        ctx.ui.notify("Review cancelled", "info")
        return
      end
      local entries = ctx.sessionManager.get_entries()
      local messageCount = 0
      for _, e in ipairs(entries) do if e and e.type == "message" then messageCount = messageCount + 1 end end
      local useFreshSession = messageCount == 0
      if messageCount > 0 then
        local choice = ctx.ui.select("Start review in:", { "Empty branch", "Current session" })
        if choice == nil then
          if fromSelector then
            target = nil
          else
            ctx.ui.notify("Review cancelled", "info")
            return
          end
        else
          executeReview(ctx, target, choice == "Empty branch", { extraInstruction = extraInstruction })
          return
        end
      else
        executeReview(ctx, target, false, { extraInstruction = extraInstruction })
        return
      end
    end
  end,
})

pi.register_command("end-review", {
  description = "Complete review and return to original position",
  handler = function(_args, ctx) runEndReview(ctx) end,
})
