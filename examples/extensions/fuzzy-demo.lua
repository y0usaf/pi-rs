-- Exercises the pi.tui fuzzy/regex mechanism bindings (pi-tui fuzzy.ts +
-- the session selector's JS-regex search): fuzzy_filter's in-order
-- character matching with best-match-first ordering and space-separated
-- token AND semantics, fuzzy_match's per-token {matches, score}, and
-- js_regex_search's `new RegExp(pattern, "i")` index/error semantics.
local pi = ...

pi.register_command("fuzzy-demo", {
  description = "Filter a provider list through pi.tui.fuzzy_filter",
  handler = function(args)
    local request = pi.json.decode(args)
    local items = {
      { id = "anthropic", name = "Anthropic" },
      { id = "openai", name = "OpenAI" },
      { id = "openai-codex", name = "OpenAI Codex" },
      { id = "openrouter", name = "OpenRouter" },
      { id = "google", name = "Google" },
    }
    local filtered = pi.tui.fuzzy_filter(items, request.query or "", function(item)
      return item.name .. " " .. item.id
    end)
    local ids = {}
    for _, item in ipairs(filtered) do ids[#ids + 1] = item.id end

    -- Single-token scoring (session-selector-search.ts matchSession).
    local match = pi.tui.fuzzy_match(request.query or "", "Anthropic anthropic")

    -- JS regex search: UTF-16 index of the first case-insensitive match,
    -- nil for no match, (nil, message) for an invalid pattern.
    local index = pi.tui.js_regex_search("open(ai|router)", "prefer OpenRouter today")
    local invalid, invalid_error = pi.tui.js_regex_search("(unclosed", "text")

    return {
      ids = ids,
      matches = match.matches,
      score = match.score,
      regexIndex = index,
      invalidIsNil = invalid == nil,
      invalidHasError = invalid_error ~= nil,
    }
  end,
})
