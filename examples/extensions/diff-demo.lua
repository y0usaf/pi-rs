-- Exerciser for the jsdiff bindings: pi.diff.lines, pi.diff.words,
-- pi.diff.unified_patch.
--
-- In pi these are the "diff" npm package (jsdiff 8.0.4): Diff.diffLines and
-- Diff.createTwoFilesPatch back the edit tool's diff details
-- (core/tools/edit-diff.ts), and Diff.diffWords backs intra-line diff
-- highlighting (modes/interactive/components/diff.ts). pi-rs exposes the same
-- library as a binding so translations stay mechanical:
--   Diff.diffWords(a, b)            → pi.diff.words(a, b)
--   Diff.diffLines(a, b)            → pi.diff.lines(a, b)
--   Diff.createTwoFilesPatch(
--     p, p, a, b, undefined, undefined,
--     { context = 4, headerOptions = Diff.FILE_HEADERS_ONLY })
--                                   → pi.diff.unified_patch(p, p, a, b,
--                                       { context = 4, headers = "file" })
-- Change objects keep jsdiff's shape: { value, count, added, removed }.
local pi = ...

pi.register_command("diff-demo", {
  description = "Walk the jsdiff line/word/patch bindings",
  handler = function()
    -- word.js dedupes the whitespace around removed words:
    -- K:'foo ' D:'bar ' K:'baz', not K:'foo ' D:' bar ' K:' baz'.
    local words = pi.diff.words("foo bar baz", "foo baz")

    local lines = pi.diff.lines("one\ntwo\nthree\n", "one\nTWO\nthree\n")

    -- The exact call shape of edit-diff.ts generateUnifiedPatch; the input
    -- has no trailing newline, so the patch carries the
    -- "\ No newline at end of file" marker.
    local patch = pi.diff.unified_patch(
      "greeting.txt", "greeting.txt",
      "Hello, world!", "Hello, testing!",
      { context = 4, headers = "file" }
    )

    return { words = words, lines = lines, patch = patch }
  end,
})
