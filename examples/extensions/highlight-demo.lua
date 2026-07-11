-- Exerciser for the highlight.js bindings: pi.hljs.highlight,
-- pi.hljs.supports_language, pi.hljs.list_languages.
--
-- In pi this is the "highlight.js" npm package (10.7.3), wrapped by
-- utils/syntax-highlight.ts and driven by theme.ts highlightCode /
-- getLanguageFromPath. pi-rs exposes the same library as a binding so
-- translations stay mechanical:
--   hljs.highlight(code, { language, ignoreIllegals })
--       → pi.hljs.highlight(code, { language = ..., ignore_illegals = ... })
--   hljs.highlightAuto(code, subset)
--       → pi.hljs.highlight(code, { language_subset = subset })
--   hljs.getLanguage(name) ~= nil → pi.hljs.supports_language(name)
-- Results keep the library's shape: { value, relevance, illegal, language },
-- where value is the hljs-class HTML span markup that
-- renderHighlightedHtml (the Lua port in the builtin packs) walks to apply
-- terminal styles.
local pi = ...

pi.register_command("highlight-demo", {
  description = "Walk the highlight.js bindings",
  handler = function()
    -- Explicit language, the theme.highlightCode path.
    local ts = pi.hljs.highlight("const x: number = 1; // note", {
      language = "typescript",
      ignore_illegals = true,
    })

    -- Aliases resolve like hljs.getLanguage: html → xml, toml → ini.
    local aliases = {
      html = pi.hljs.supports_language("html"),
      toml = pi.hljs.supports_language("toml"),
      quux = pi.hljs.supports_language("quux"),
    }

    -- Auto-detection over a subset (array subLanguage modes use this).
    local auto = pi.hljs.highlight('{"a": [1, 2, 3]}', {
      language_subset = { "json", "python" },
    })

    return {
      ts_value = ts.value,
      ts_relevance = ts.relevance,
      aliases = aliases,
      detected = auto.language,
      languages = #pi.hljs.list_languages(),
    }
  end,
})
