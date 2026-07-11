# pi-rs — execution plan

`DESIGN.md` defines the target: an indistinguishable Rust port of Pi v0.79.0's
coding agent. `ref/pi` @ `c5582102` is the specification. This plan covers the
coding agent and its required AI/auth, agent, and TUI machinery—nothing else Pi
ships. Product-specific work belongs in separate downstream forks.

The first unchecked item is the next task. Product behavior lands as embedded
Lua through the public extension surface (DESIGN divergence 2); Rust is
mechanism only. Do not preserve known differences as "temporary" UI.

## Proven foundation

Implementation inventory, not claims of Pi parity:

- [x] Rust workspace and Nix build/check foundation exist.
- [x] Lua host supports coroutine handlers, registries, discovery/trust, OS
      bindings, and watchdog isolation.
- [x] AI types, HTTP/SSE transport, Anthropic + openai-completions protocols,
      catalog-as-data (`data/models.json`), PKCE auth engine, and deterministic
      protocol fixtures exist.
- [x] Coding tools, Lua agent loop, tool round trips, JSONL session writes,
      one-shot CLI, and an interactive vertical slice exist — the interactive
      transcript still renders placeholder `you:`/`assistant:` chrome that
      item 2 removes.
- [x] Terminal cell rendering, input decoding, editor/autocomplete/markdown/
      layout components, and the async process driver exist.

## Current milestone — exact interactive shell

- [x] **1. Add the Pi differential UI harness.** `scripts/ui-diff` replays the
      shared `tests/ui-parity/basic-turn.json` scenario through Pi-derived
      components and pi-rs's public Lua/TUI seam, then compares terminal-emulated
      cells (including trailing blanks, colors/attributes, wide cells, geometry,
      cursor state, and checkpoint order). `scripts/ui-diff --update-pi`
      regenerates the checked-in oracle from `ref/pi`; the normal offline command
      reports the first cell/cursor mismatch with row context. The fixture covers
      startup, submitted input, assistant streaming/completion, and resize. A
      negative-control test proves a `you:` prefix fails at cell `(0,0)`.

      **Evidence:** `cargo test -p pi-rs-tui ui_harness`; `scripts/ui-diff` reports
      the current startup cursor mismatch actionably. The mismatch is expected
      until item 2 replaces the placeholder renderer; do not update the oracle
      from pi-rs.

- [x] **2. Replace the placeholder transcript renderer.** Port Pi's
      user-message, assistant-message, thinking, tool-call/result, and streaming
      presentation into the Lua frontend. Remove `you:`/`assistant:` labels and
      all other pi-rs-only chrome, and delete the tests that expected them. Match
      spacing, padding, Markdown, colors, wrapping, and cursor placement.

      **Evidence:** `scripts/ui-diff` reports "Pi/pi-rs UI cells match at 6
      checkpoints" (startup/submitted/streaming/complete at 72 cols, resizes to
      48 and 100), including cursor row/column/visibility and bold/color
      attributes — the oracle is regenerated with `FORCE_COLOR=3` so chalk
      styling matches real terminals. Mechanism ported 1:1 into Rust:
      `tui.rs` doRender (per-line `2K` rewrites, SEGMENT_RESET line resets,
      viewport/hardware-cursor tracking), `utils.rs` `wrapTextWithAnsi` +
      `AnsiCodeTracker`, and `markdown.rs` (full markdown.ts render pipeline
      with theme/default-style/marker options over a marked-approximating
      lexer). Policy ported into `interactive.lua`: user-message Box+Markdown
      with `userMessageBg`/`userMessageText` and OSC 133 zones,
      assistant-message spacing/thinking/aborted/error rules, streaming via
      the agent's partial assistant message, tool executions on the
      tool-execution.ts generic-fallback path, chalk-semantics
      bold/italic/underline/strikethrough, and `getMarkdownTheme`. Old
      `you:`/`assistant:` tests replaced by
      `frontend_frame_mounts_pi_message_components_without_pi-rs_chrome` which
      forbids the labels. New public surface (`markdown_render` opts,
      `pi.json.encode`) exercised by `examples/extensions/tui-markdown-demo.lua`
      + `pi-rs-host tests/tui_markdown.rs`. `cargo test --workspace`: 488 passed.

**2b. Extend transcript parity to rich content** — the parts of Pi's
transcript presentation the basic-turn fixture does not pin, split into
landable rungs:

- [x] **2b.1 jsdiff-parity diff mechanism.** `pi.diff.lines`, `pi.diff.words`,
      and `pi.diff.unified_patch` (crates/pi-rs-host/src/jsdiff.rs) port jsdiff
      8.0.4 — Pi's `diff` dependency — 1:1: the Myers loop with jsdiff's
      edge-clamping and tie-breaking, line/word tokenization (JS `\s`,
      `extendedWordChars`), word-join semantics, whitespace dedupe
      post-processing, and `structuredPatch`/`formatPatch` with
      include/file/omit headers. Replaced the divergent naive positional diff
      in `edit-diff.lua`: `generate_diff_string` is now the exact
      `generateDiffString` port over `pi.diff.lines`, and
      `generate_unified_patch` is `createTwoFilesPatch(path, path, …,
      {context 4, FILE_HEADERS_ONLY})`.

      **Evidence:** `tests/jsdiff-parity/` oracle generated from the vendored
      library (`scripts/jsdiff-oracle`) replays through the public Lua surface
      (`cargo test -p pi-rs-host --test jsdiff_parity`); two 300–400-case random
      differential fuzz rounds against vendored jsdiff found no divergence
      (empty-table vs `[]` JSON encoding artifact aside);
      `edit_diff_details_match_pi_jsdiff_shapes` pins the edit tool's
      diff/patch/firstChangedLine against output verified byte-identical to
      `ref/pi` `edit-diff.ts`; `examples/extensions/diff-demo.lua` exercises
      the new hook. `cargo test --workspace` green; `scripts/ui-diff` still
      matches at 6 checkpoints.

- [x] **2b.2 Per-tool renderers.** `renderCall`/`renderResult` for read,
      bash, edit, write, grep, find, ls ported onto the Lua tool definitions
      (renderers return `function(width) -> lines` components), plus the
      support ports: `tools/render-utils.lua` (str/shortenPath/renderToolPath/
      getTextOutput/getLanguageFromPath + text/box/spacer component helpers),
      `tools/keybinding-hints.lua` keyText/keyHint over the default bindings,
      `tools/visual-truncate.lua`, and `tools/diff.lua` renderDiff with
      `pi.diff.words` intra-line inverse (theme.inverse added to create_theme).
      `tool_execution_lines` in interactive.lua is now the full
      tool-execution.ts port: registry-resolved definitions, default Spacer +
      Box(1,1,statusBg) vs `renderShell = "self"`, pending/success/error
      backgrounds, call/result fallbacks, generic no-definition fallback;
      edit previews run `compute_edits_diff` (edit-diff.lua) inline. Bash
      Elapsed/Took reads `context.now_ms` — the frontend passes the real
      clock, parity fixtures a scripted one (oracle patches `Date.now`).
      Image blocks defer to the images milestone; syntax highlighting takes
      the unvalidated-language fallback until 2b.3.

      **Evidence:** `tests/ui-parity/tool-turn.json` drives Pi's real
      `ToolExecutionComponent` + core/tools definitions (`pi-tool-turn.ts`,
      with the coding-agent KeybindingsManager and pinned capabilities) and
      pi-rs's `interactive-tool-parity-sequence`; `scripts/ui-diff` reports
      "Pi/pi-rs UI cells match at 10 checkpoints" (read/grep/grep-error,
      bash pending→partial→results, edit preview-diff + edit-error, each
      collapsed and expanded) alongside basic-turn's 6, including the
      intra-line inverse cells and Elapsed/Took rows. New renderer hook
      exercised by `examples/extensions/tool-render-demo.lua` +
      `tool_render_example_drives_custom_renderers_through_the_transcript`.
      `cargo test --workspace`: 492 passed.

- [x] **2b.3 Syntax highlighting.** highlight.js 10.7.3 — Pi's pinned
      `highlight.js` dependency — ported on the jsdiff split: the parse
      engine (`crates/pi-rs-host/src/hljs/`: core.js `_highlight` loop,
      MultiRegex/ResumableMultiRegex with `join`'s backref renumbering and
      the shared mutable rule positions, TokenTree/HTML emitter, keyword
      engine, sublanguage recursion + `highlightAuto`, SAFE_MODE, the four
      named grammar callbacks, JS→fancy-regex source translation) is Rust
      mechanism behind `pi.hljs.highlight/supports_language/list_languages`;
      grammars are data compiled by the vendored library itself
      (`scripts/hljs-grammars` → `crates/pi-rs-host/data/hljs-grammars.json`).
      Pi's own layer is Lua: `utils/syntax-highlight.lua` (shared fragment in
      both builtin packs) ports `renderHighlightedHtml` + `decodeHtmlEntityAt`,
      `buildCliHighlightTheme`, and both `highlightCode` variants; the
      read/write renderers and `getMarkdownTheme` now highlight for real.

      **Boundary (recorded):** the catalog holds 41 grammars — every
      `getLanguageFromPath` target the library ships plus the fence tags Pi's
      tests pin (`diff`, `html`), closed over sublanguage references. Pi
      registers all 191; fence tags outside the set take the unvalidated-
      language mdCodeBlock fallback in pi-rs where Pi would highlight. Grammars
      whose runtime callbacks are not the four named ones cannot enter the set
      (gen-grammars.ts errors); auto-detect competes over the scoped set only.

      **Evidence:** `tests/hljs-parity/` oracle (53 cases, all 41 languages +
      auto subsets) generated from the vendored library replays through the
      public Lua surface byte-identically (`cargo test -p pi-rs-host --test
      hljs_parity`); a one-off 2173-case cross sweep (every snippet × every
      language) found zero divergences. New ui-parity scenarios
      `highlight-turn` (fenced ```typescript/```python/unknown-tag blocks,
      streaming mid-fence, resize) and `highlight-tool-turn` (read `.ts`,
      write `.py`, collapsed + expanded) match Pi cell-for-cell —
      `scripts/ui-diff` now reports 6+10+5+6 checkpoints. New hook exercised
      by `examples/extensions/highlight-demo.lua`.

## Pulled-forward milestone — usable interactive auth and model choice

Reprioritized 2026-07-09: the interactive product cannot `/login` or switch
models, which blocks daily driving — typing `/login` is sent to the model as
a prompt. The bare-core CLI path (`pi --login`, `--list-models`, `--model`,
env keys, settings defaults) already works; this milestone ports Pi's
in-product surface. Consistent with DESIGN delivery order 3 ("one provider
and auth path end-to-end"); the remaining 2b transcript rungs do not block
it and move behind it. Spec anchors: `interactive-mode.ts` handleSubmit's
command routing, `components/oauth-selector.ts`, `components/login-dialog.ts`,
`components/model-selector.ts`, `core/auth-storage.ts` (Rust port exists),
`core/model-registry.ts` (Rust port exists).

- [x] **3a.1 Slash-command dispatch + selector overlay machinery.** Ported
      into `interactive.lua`: `handle_submit` — the `setupEditorSubmitHandler`
      routing skeleton (trim, `editor.setText("")`, `/quit` → shutdown,
      extension/unknown "/" fallthrough to the prompt path) wired into
      `run_interactive`; `show_selector` — the `showSelector` editor-slot
      swap with focus move and `done()` restore (frontend_frame renders the
      selector in place of the editor, input routes to it); and the selector
      template itself — `oauth_selector` ports oauth-selector.ts presentation
      and input policy 1:1 (DynamicBorder/TruncatedText/Spacer/Input
      composition, tui.select.* bindings, maxVisible-8 windowing + scroll
      info, empty/no-match messages, all formatStatusIndicator branches) with
      data callbacks injected; the auth bridge and /login wiring are 3a.2.
      New mechanism binding: `pi.tui.fuzzy_filter` (pi-rs-tui fuzzy port).
      Other builtin commands stay unrouted until their rungs — Pi has no
      pre-dialog behavior for them.

      **Evidence:** `tests/ui-parity/selector-turn.json` drives Pi's real
      `OAuthSelectorComponent` through the showSelector swap
      (`pi-selector-turn.ts`) and pi-rs's product machinery
      (`interactive-selector-parity-sequence`); `scripts/ui-diff` reports
      "Pi/pi-rs UI cells match at 7 checkpoints" (startup editor, open,
      down×6 windowing, fuzzy filter, no-match, cancel-restore, typing after
      restore — including the search-input cursor cells and marker).
      `submit_router_matches_pi_command_interception` pins /quit +
      fallthrough semantics;
      `selector_overlay_confirms_and_cancels_through_show_selector` pins
      confirm/cancel + editor-slot restore. `pi.tui.fuzzy_filter` exercised
      by `examples/extensions/fuzzy-demo.lua` + `tui_fuzzy.rs`.
      `cargo test --workspace`: 499 passed.

- [x] **3a.2 `/login` and `/logout` end-to-end.** Mechanism: `auth-storage.ts`
      + `resolve-config-value.ts` ports moved pi-rs-app→pi-rs-host (re-exported;
      one instance per VM) and bound as `pi.auth.*` (get/set/remove/list,
      auth status, oauth provider mirror, env/config-value resolution,
      auth_path, runtime keys); `pi.auth.login_start(provider)` spawns the
      OAuth flow on the VM runtime and bridges `OAuthLoginCallbacks` as an
      event-stream handle (`next_event`/`respond`/`cancel`, cancel = the
      spec's "Login cancelled"), persisting on success like
      `authStorage.login`; `pi.ai.providers()`, `pi.platform()`, and
      `pi.open_browser()` (open-browser.ts, no shell) round out the seam.
      Policy in interactive.lua: login-dialog.ts and extension-selector.ts
      ports, showStatus/showError/showWarning transcript rows,
      provider-display-names data, getLogin/getLogoutProviderOptions,
      isApiKeyLoginProvider, showLoginAuthTypeSelector → provider selector →
      oauth/api-key/Bedrock dialogs, pump_login event dispatch,
      completeProviderAuthentication's reachable slice + the anthropic
      subscription warning; `/login` `/logout` routed in handle_submit.
      Deferred, recorded in code: registry refresh/provider-count/footer
      invalidation and the unknown-model default-model branch (3a.3),
      settings gate for the warning (item 5), oauth breadth beyond
      anthropic (item 8).

      **Evidence:** `tests/ui-parity/login-turn.json` drives pi's real
      LoginDialogComponent/ExtensionSelectorComponent/OAuthSelectorComponent
      through the interactive-mode wiring bodies (`pi-login-turn.ts`) and
      pi-rs's product wiring (`interactive-login-parity-sequence`);
      `scripts/ui-diff` reports "Pi/pi-rs UI cells match at 24 checkpoints"
      (auth-type selector, provider selector with configured/subscription
      indicators, oauth dialog URL/manual-input/typing/submit/progress/done,
      status + subscription-warning rows, Bedrock info dialog, api-key
      prompt/save/empty-error, logout selector/confirm, cancel with
      suppressed "Login cancelled") alongside the prior 6+10+5+6+7.
      `interactive_login.rs` completes a real PKCE flow against a stubbed
      token endpoint through `/login` → selectors → dialog → auth.json
      (`{type:"oauth",access,refresh,expires}`), then `/logout` clears it.
      `pi.auth` exercised by `examples/extensions/auth-demo.lua` +
      `pi-rs-host tests/auth_bindings.rs` (CRUD, login bridge event order,
      cancel, unknown provider). `--login` CLI path unchanged
      (`cli/login.rs` still consumes the moved AuthStorage).
      `cargo test --workspace`: 501 passed.

- [x] **3a.3 `/model` and the model selector.** Mechanism: the
      `model-registry.ts` port moved pi-rs-app→pi-rs-host (storage handle passed
      explicitly — divergence recorded in the module doc — so one per-VM
      `AuthStorage` is shared by `pi.auth`, the new `pi.ai` registry bridge
      (`registry_refresh`/`registry_error`/`available_models`/`find_model`/
      `has_configured_auth`/`is_using_oauth`), and login flows; the shared
      storage became a tokio mutex with async `pi.auth` bindings so the new
      `pi.auth.get_api_key` (runtime → stored → OAuth-refresh → env) can
      await refreshes). The agent loop's existing `getApiKey` seam now
      resolves each request's key for the *current* model's provider;
      `--api-key` reaches the VM as `runtimeApiKey` → `set_runtime_api_key`.
      Policy in interactive.lua: `model-selector.ts` port (scope all/scoped +
      tab toggle, wrap-around navigation, maxVisible-10 windowing + scroll
      info, current-model ✓, Model Name row, error/no-match rows, the spec's
      fuzzy key string), `findExactModelReferenceMatch` port,
      `handleModelCommand`, `showModelSelector`, `cycleModel` (scoped
      auth-filtered, wrap, "Only one model…" statuses), and
      `updateAvailableProviderCount` (startup, login completion, logout —
      closing the 3a.2 deferrals); footer gained `formatCwdForFooter` and the
      provider-count/subscription plumbing. `/model [search]` routed in
      handle_submit; ctrl+l / ctrl+p / shift+ctrl+p wired as editor actions.
      Deferred, recorded in code: settings `setDefaultModelAndProvider`
      (settings bridge, items 5/9), session `appendModelChange` (item 6),
      thinking re-clamp (no-op until levels land, item 7), daxnuts easter egg
      and `/scoped-models` (item 7), `/model` argument autocomplete (item 3).

      **Evidence:** `tests/ui-parity/model-turn.json` drives pi's real
      `ModelSelectorComponent` + `FooterComponent` through the
      interactive-mode wiring bodies (`pi-model-turn.ts`) and pi-rs's product
      wiring (`interactive-model-parity-sequence`); `scripts/ui-diff` reports
      "Pi/pi-rs UI cells match at 15 checkpoints" (open, up-wrap windowing,
      fuzzy filter, no-match, select → footer + status row, exact `/model
      provider/id`, ambiguous `/model claude` prefill, cancel, cycle
      forward/backward with in-place status update, scoped open/tab/tab-back/
      cancel — including the ~-substituted cwd line and the footer's
      truncated right side) alongside the prior 6+10+5+6+7+24.
      `interactive_model.rs` pins that the next provider request after
      `/model openai/gpt-5.4` streams with that model and that provider's
      stored API key via the per-request seam. New hooks exercised by
      `examples/extensions/model-registry-demo.lua` + `pi-rs-host
      tests/model_registry_bindings.rs`. `cargo test --workspace`: 503
      passed.

## Deferred transcript and shell rungs

- [x] **2b.4 Marked edge-case conformance.** Pinned the hand-rolled markdown
      lexer against Pi's marked 15.0.12 pipeline on a markdown-heavy fixture
      and fixed the divergences it caught: `***x***` now lexes as em wrapping
      strong (marked emStrong), setext underlines (`===`/`---`) are checked
      before paragraph interrupts (marked's lheading beats hr), and the
      renderer gained the missing `getCapabilities().hyperlinks` OSC 8 branch
      from markdown.ts. Capabilities are pinned deterministically on both
      harness sides (ui-diff.rs `set_capabilities`, pi-markdown-turn.ts
      `setCapabilities`: no images, truecolor, no hyperlinks).

      **Evidence:** `tests/ui-parity/markdown-turn.json` (ATX + setext
      headings, heading codespan, nested ordered/unordered/task lists, deep
      nesting, strict-strikethrough positive/negative, `***both***`,
      intraword `_`, escapes, hard break, double-backtick codespan, link
      text≠href, bare-URL/www/mailto/angle autolinks, multi-paragraph quote,
      hr, table with inline styling, untagged fence, streaming cut mid-fence,
      resizes to 48/100) drives Pi's real components (`pi-markdown-turn.ts` →
      basic-turn driver) and pi-rs's `interactive-parity-sequence`;
      `scripts/ui-diff` reports 6 markdown-turn checkpoints alongside the
      prior suites (6+10+5+6+7+24+15). Negative control: stashing the lexer
      fixes fails at the streaming checkpoint. Lexer behaviors also pinned by
      `triple_emphasis_is_em_wrapping_strong` and
      `setext_underline_beats_hr_and_list_interrupts`. No new public hooks —
      no example needed. `cargo test --workspace`: 505 passed.

- [x] **2b.5 Tool-args JSON key order.** Resolved by preserving wire order
      through the boundary (no divergence row needed). Mechanism in
      `convert.rs`: serde_json `preserve_order` workspace-wide (Pi's JS
      runtime keeps insertion order everywhere; sorted maps were a Rust-ism);
      `json_to_lua` records each object's JS [[OwnPropertyKeys]] order —
      canonical array indices ascending, then wire order — in a metatable
      field, and `lua_to_json` replays it (Lua-added keys follow as a sorted
      remainder, the old deterministic order). Numbers now follow
      JSON.parse/stringify semantics: integral doubles print without a
      fraction (`1.0` → `1`), non-finite → `null`.

      **Evidence:** tool-turn gained a `generic` section (unregistered
      `fetch_page`, multi-key args with `"10"`/`"2"` index keys, nested
      object, `retries: 1.0`) driving Pi's real generic-fallback
      `formatToolExecution` — `scripts/ui-diff` reports 13 tool-turn
      checkpoints (was 10) alongside the other suites (6+6+5+6+7+24+15);
      negative control: reverting the mechanism fails at `generic-pending`
      cell (5,4) where alphabetical order puts `"10"` before `"2"`.
      `pi-rs-host tests/json_boundary.rs` pins decode→encode byte-equality
      with node 22 `JSON.stringify` (pretty + compact), Lua-added-key
      placement, and JS number semantics. No new public hooks — existing
      `pi.json.*` surface, so no new example. `cargo test --workspace`:
      508 passed.

**3. Port the editor and interaction shell** — match Pi's editor layout,
multiline behavior, selection/history, autocomplete, keybindings, status
area/footer, focus, paste handling, Escape behavior, and resize/repaint
semantics — all routed through the Lua frontend and public bindings. Split
into rungs:

- [x] **3.1 Editor core parity.** `tests/ui-parity/editor-turn.json` drives
      Pi's real `CustomEditor` (coding-agent KeybindingsManager +
      `getEditorTheme`) through input scripts (`pi-editor-turn.ts`) and pi-rs's
      product `custom_editor` over the `pi.tui.editor` mechanism
      (`interactive-editor-parity-sequence`). Mechanism fixes found and
      pinned: bordered render rewritten to editor.ts `render` (borderMuted-
      styled borders via new `set_border_style` binding, paddingX layout with
      cursor-in-padding, scroll indicators `─── ↑/↓ N more`, autocomplete
      rows inside content width); newline handling (shift+enter/`\n`/
      `\x1b\r`/`\x1b[13;2~`/ESC+CR forms), backslash-enter newline on submit,
      `submitValue` semantics (always clears and fires, even empty; no
      mechanism-side history — history is app policy, added on the prompt
      path per interactive-mode.ts), disable_submit enter is a no-op,
      shift+space/shift+backspace/shift+delete, key decode fixes (`\x1f`
      ctrl+-, legacy pageUp/pageDown, ctrl+alt+letter, ctrl+\\, shift+tab),
      large-paste marker counts JS `String.length` and total lines
      (`+12 lines` for a 12-line paste), history navigation resets scroll.
      Product wiring: editor gets the theme border + focus (hardware-cursor
      marker), `set_editor_focus` real, prompt-path `add_to_history`.

      **Evidence:** `scripts/ui-diff` reports "Pi/pi-rs UI cells match at 21
      checkpoints" (typing+wrap with shift+space, word/jump moves, kill/yank/
      undo, multiline, backslash-newline, submit scaffold rows, history
      up/walk/exit-to-draft, atomic paste marker insert/delete, 10-line
      scroll + pageUp/pageDown indicators, empty-submit clear, re-wrap at
      40/100 cols — cursor row/col/visibility included) alongside the prior
      suites. Negative control: reverting the `\x1f` decode fails at
      "undone" with a cursor diff. `tui-multiline-editor-demo.lua` updated to
      the app-policy history and disable_submit no-op seams. `cargo test
      --workspace`: 508 passed.

- [x] **3.2 Autocomplete parity.** `CombinedAutocompleteProvider` ported 1:1
      as mechanism (crates/pi-rs-tui/src/autocomplete.rs — prefix/quote/@ token
      extraction, node-path display semantics, `buildCompletionValue`
      quoting, readdir completion with symlink-dir stat + dirs-first locale
      sort, fd walk with the spec's exact args/`buildFdPathQuery`/.git
      filtering/`scoreEntry` top-20, and every `applyCompletion` branch),
      exposed as `pi.tui.autocomplete_provider` with per-command
      `get_argument_completions` Lua callbacks. `SelectList` rewritten to the
      select-list.ts layout (primary column widest+2 clamped, description
      column at width>40, scroll info, themed slots); the editor injects
      `getSelectListTheme` styles and the slash [12,32] layout clamp. Editor
      trigger policy now matches insertCharacter/handleBackspace/
      handleForwardDelete/moveCursor exactly (char-class gates, token
      boundaries, `[\s]`-pattern, ctrl+c passthrough instead of cancel, no
      re-trigger on kills/jumps/paste); requests carry the spec's 20ms
      debounce, honored by the interactive pump on ticks. interactive.lua
      wires BUILTIN_SLASH_COMMANDS (22 commands), /model argument
      completions (fuzzy over "<id> <provider>"), fd/fdfind PATH probe
      (ensureTool's system slice; release-download fallback deferred to the
      tools milestone), and the request pump in run_interactive. Prompt
      templates, extension commands, skill commands, and extension provider
      wrappers join with items 7/9.

      **Evidence:** `tests/ui-parity/autocomplete-turn.json` drives Pi's
      real `CombinedAutocompleteProvider` + `CustomEditor` with the
      createBaseAutocompleteProvider wiring body (`pi-autocomplete-turn.ts`)
      against pi-rs's product machinery
      (`interactive-autocomplete-parity-sequence`) over a shared scenario
      tree and a deterministic `fd-stub`; `scripts/ui-diff` reports "Pi/pi-rs
      UI cells match at 19 checkpoints" (slash open/scroll windowing/fuzzy
      filter/tab apply, /model argument menu + best-match selection +
      confirm, single-result tab auto-apply, multi-result tab menu + escape,
      @-fuzzy open/filter/apply with descriptions, quoted-directory apply
      with cursor inside the quotes, resize with open menu) alongside the
      prior suites (6+6+21+13+5+6+7+24+15). Negative control: perturbing the
      scroll-info format fails at slash-open cell (8,4). Mechanism pinned by
      autocomplete.ts-test ports in Rust (`cargo test -p pi-rs-tui
      autocomplete`); new hook exercised by
      `examples/extensions/tui-autocomplete-demo.lua` + `pi-rs-host
      tests/tui_autocomplete.rs`. `cargo test --workspace`: 521 passed.

- [x] **3.3 Interaction shell.** interactive-mode.ts shell semantics ported
      into `interactive.lua`: init()'s container composition (headerContainer
      Spacer+ExpandableText+Spacer, chatContainer, pendingMessagesContainer,
      statusContainer, widget-above default spacer, editorContainer, footer)
      as `frontend_frame`; `setup_shell_editor`/`shell_submit_actions` (the
      setupKeyHandlers + setupEditorSubmitHandler wiring shared by the
      product loop and the parity sequence); escape-abort with
      `restoreQueuedMessagesToEditor({abort})` + double-escape timing gate
      (tree/fork targets deferred to item 6, recorded in code); handleCtrlC
      press-again-within-500ms exit (pi-rs-only "Press ctrl+c again" chrome
      removed) and ctrl+d shutdown; the working Loader on
      agent_start/agent_end (spinner mechanism `pi.tui.loader`); queued
      steering/follow-up text tracking with message_start consumption,
      pending-message rows + ↳ Alt+Up hint, alt+enter follow-up, alt+up
      dequeue; ctrl+o header/tools expansion; ctrl+v paste-image insert (OS
      clipboard mechanism deferred to the images milestone, seam recorded);
      and extension shortcuts via the new `pi.register_shortcut` /
      `pi.registered_shortcuts` host mechanism. Mechanism fixes: keys.ts
      modifier arrow/home-end decode (`\x1b[1;<mod>A/B/C/D/H/F`, alt+up),
      JS-split key formatting ("/" hint). The "Thinking…" status stand-in is
      gone — basic-turn now pins the real Loader, and basic/selector drivers
      use the real CustomEditor on both sides.

      **Evidence:** new `tests/ui-parity/shell-turn.json` drives pi's real
      composition + copied interactive-mode handler bodies
      (`pi-shell-turn.ts`) against pi-rs's product wiring
      (`interactive-shell-parity-sequence`); `scripts/ui-diff` reports
      "Pi/pi-rs UI cells match at 13 checkpoints" (compact/expanded/collapsed
      header hints, typed, ctrl+c clear, submitted + ⠋ Working... loader,
      steer/follow-up queue rows, escape-abort restoring both texts into a
      multiline editor with "Operation aborted", paste-image insert,
      shortcut status row, dequeue-empty in-place status update, resize) —
      all suites now 6+6+21+19+13+13+5+6+7+24+15 with regenerated oracles.
      Negative control: dropping handleCtrlC's clear fails at
      "ctrl-c-clear" with a cursor diff.
      `shell_sequence_pins_queue_restore_abort_and_press_again_exit` pins
      prompt/steer/followUp/abort event order, editor restore, and
      double-ctrl+c exit; `pi.register_shortcut` exercised by
      `examples/extensions/shortcut-demo.lua` +
      `shortcut_example_fires_through_registered_shortcuts` and host
      registry tests. `cargo test --workspace`: 525 passed.

      **Accept (whole item):** shared input scripts produce the same frames,
      cursor positions, submitted prompts, and cancellation behavior in Pi
      and pi-rs. ✓ via shell-turn + the updated basic/selector turns.

## Next milestone — exact coding-agent loop

**4. Match one complete provider/auth path** — finish Anthropic end to end
as observed by the coding agent (3a landed the `/login` and model-selection
surface; this item closes the transport-level behavior): API-key credentials,
model resolution, requests, streaming events, usage, errors, retry, and
cancellation. Split into rungs:

- [x] **4.1 Anthropic protocol differential oracle.** `scripts/anthropic-oracle`
      runs Pi's real `streamAnthropic`/`streamSimpleAnthropic` (vendored
      `@anthropic-ai/sdk` 0.91.1) against a scripted local HTTP stub and
      records, per case, every captured HTTP request (method, path, meaningful
      headers, body), the emitted event sequence (minus partial snapshots),
      and the final message into `tests/anthropic-parity/oracle.json`;
      `cargo test -p pi-rs-ai --test anthropic_parity` replays the same
      `cases.json` through pi-rs's public protocol surface and compares.
      Divergences the oracle exposed, fixed as mechanism: anthropic now runs
      the SDK retry loop (`RetryPolicy::AnthropicSdk` — `x-should-retry`
      override, 408/409/429/5xx predicate, retry-after/retry-after-ms delays
      uncapped, jittered backoff, no body read before a retry decision) beside
      the codex loop; HTTP failures surface the SDK's `APIError.makeMessage`
      strings (`400 {json}`, `529 Overloaded!`, `500 status code (no body)`);
      abort mid-read surfaces undici's "This operation was aborted"
      (`TransportError::BodyAborted`) while loop-top checks keep "Request was
      aborted"; serde_json gained `float_roundtrip` so JSON floats parse with
      JS semantics.

      **Evidence:** 29 oracle cases match — request shaping (api-key full,
      oauth identity + tool-name mapping, thinking budget/adaptive/disabled,
      fireworks compat + session affinity, cache retention, interleaved-off,
      simple-options mapping), streaming (rich text/thinking/redacted/tool
      transcript with split json deltas, usage propagation, stop-reason
      mapping incl. refusal/pause_turn/unknown), errors (SSE error event,
      HTTP JSON/non-JSON/empty bodies, truncated stream, missing key sync +
      stream flavors), retry (500/409/x-should-retry-false/x-should-retry-
      true-400/exhausted with captured request counts), cancellation
      (abort-mid-stream: aborted stopReason, partial content, undici message).
      Negative control: reverting to the codex retry policy fails at
      `retry-should-retry-false`. Hand-derived
      `fixtures/anthropic` expectations + `tests/anthropic.rs` ablated
      (superseded); `replay_basic.sse` kept as stub input for
      `anthropic_replay.rs`. `cargo test --workspace`: 517 passed;
      `scripts/ui-diff` still matches at all 11 suites.

- [x] **4.2 Provider-turn UI parity.** `tests/ui-parity/provider-turn.json`
      drives both real stacks against the same scripted SSE stub: pi's real
      `Agent` + `streamAnthropic` + real coding tools with the copied
      interactive-mode handleEvent bodies (`pi-provider-turn.ts`, node stub
      in-process), and pi-rs's product machinery — `create_interactive_state`
      (the run_interactive wiring, refactored out) with the real Lua agent
      loop, registered tools, and `pi-rs-ai` anthropic protocol (ui-diff
      serves the stub, injects `model.baseUrl`/cwd/HOME, pins
      `PI_CODING_AGENT_DIR` to an empty temp dir). Frames are captured at
      exact agent-event points on both sides (pi: awaited listener; pi-rs:
      the synchronous subscribe hook); mid-stream checkpoints pace the pi
      stub (`pauseAfter` → capture gates) because pi's provider eagerly
      consumes buffered SSE and mutates the shared partial. Mechanism
      landed: `pi.spawn` background coroutines (LocalSet-driven dispatch;
      the interactive session runs turns concurrently with its event loop,
      as `void session.prompt()` does) and the watchdog budget now bounds
      *continuous* Lua execution — every host await resets the window, so
      a long-lived agent-loop dispatch survives. Policy ported into
      interactive.lua: the interactive-mode handleEvent bodies (streaming
      assistant row updated in place, pending tool rows mounted from
      toolCall blocks at message_update, aborted → "Operation aborted" +
      pending-tool error settlement, setArgsComplete, agent_end cleanup,
      user-message Spacer-when-chat-nonempty), and the FooterComponent
      data path (cumulative usage totals, latest-entry cache hit rate with
      the known-absent gate, `getContextUsage` percent over the
      compaction.ts token-estimation slice, JS-length chars/4).

      **Evidence:** `scripts/ui-diff` reports "Pi/pi-rs UI cells match at 14
      checkpoints" (startup, working loader, mid-stream text, completed
      turn, mid-stream tool-call args, tool pending/executed/complete,
      expanded + collapsed tool output, mid-stream cancel + aborted
      settlement, HTTP-400 error turn, resize to 100 cols — footer usage/
      cache/context cells included) alongside the prior suites
      (6+6+21+19+13+13+5+6+7+24+15). Negative control: disabling the
      message_update tool-row mount fails at "tool-streaming" (0,0).
      `pi.spawn` exercised by `examples/extensions/spawn-demo.lua` +
      `pi-rs-host tests/parallel.rs` and pinned by seam tests
      (`spawned_task_interleaves_with_the_dispatching_handler`,
      `awaits_reset_the_continuous_execution_window`). `cargo test
      --workspace`: 520 passed; `nix flake check` green.

      **Accept (whole item):** recorded fixtures produce equivalent requests
      and event sequences (4.1 oracle, 29 cases), and both applications
      render the same successful, tool-using, cancelled, and failed turns
      (provider-turn, 14 checkpoints). ✓

**5. Match agent and tool semantics** — steering, follow-ups, tool-call
ordering, read/bash/edit/write behavior, truncation, images, system prompt,
context construction, and settlement rules required by Pi. Split into rungs:

- [x] **5.1 System prompt + LLM context construction.** Ported as shared
      Lua fragments loaded by both product packs:
      `utils/system-prompt.lua` (system-prompt.ts buildSystemPrompt, the
      resource-loader.ts loadContextFileFromDir/loadProjectContextFiles
      slice, skills.ts formatSkillsForPrompt/escapeXml, and
      agent-session.ts's _normalizePromptSnippet/_normalizePromptGuidelines
      + _rebuildSystemPrompt composition over `pi.registered_tools()`) and
      `utils/messages.lua` (messages.ts convertToLlm + bashExecutionToText
      + summary prefixes). Product wiring: interactive
      `create_interactive_state` and one-shot `pi-rs-run` now build the base
      system prompt, pass `convertToLlm`, and give the agent sdk.ts's
      default active tool set read/bash/edit/write (grep/find/ls stay
      registered for rendering but inactive until the tools surface,
      item 7). Mechanism data: main.rs passes agentDir/readmePath/docsPath/
      examplesPath (config.rs gained getReadmePath/getExamplesPath).
      Deferred, recorded in code: the sdk.ts blockImages filter (settings
      bridge, items 5.3/9), custom --system-prompt/--append-system-prompt
      resolution and skills loading (items 7/9).

      **Evidence:** `tests/system-prompt-parity/` oracle (10 session + 5
      raw cases: tool subsets/unknown names, snippet gaps, guideline
      dedupe/trim, context-file candidate precedence + ancestor ordering +
      agent-dir dedupe, custom/append prompts, skills XML escaping and the
      read-tool gate, windows cwd normalization, fixed date) generated
      from pi's real buildSystemPrompt/tool definitions via
      `scripts/system-prompt-oracle` replays byte-identically through the
      public `system-prompt-parity` seam (`cargo test -p pi-rs-app --test
      system_prompt_parity`); negative control: perturbing the "Be concise
      in your responses" guideline fails at default-tools.
      `anthropic_replay.rs` pins the wired path end-to-end: both captured
      HTTP requests carry the built prompt in `system[0].text` and tools
      read/bash/edit/write. No new public hooks — no example needed.
      `cargo test --workspace`: 521 passed; `scripts/ui-diff`: all 12
      suites match.

- [x] **5.2 Tool-semantics differential oracle.** `tests/tool-parity/`
      oracle (63 cases) generated from pi's real `core/tools/` definitions
      (`scripts/tool-oracle` → gen-oracle.ts runs prepareArguments +
      execute with the agent loop's exact invocation shape over shared
      fixture trees) replays through the public Lua surface — the
      `tool-parity` command in coding-agent.lua mirrors agent.lua's
      prepare → validate → execute(id, args, signal, on_update, ctx) with
      a controllable abort ("pre" / abortAfterMs via pi.spawn) and
      injectable ctx.model — comparing results, details (full truncation
      shapes), error strings, filesystem effects, and bash's persisted
      full output byte-for-byte ({ROOT}/{FULL_OUTPUT} substituted).
      Deferrals closed: `pi.exec` gained `options.signal` (core/exec.ts:
      abort shares the timeout's SIGTERM→SIGKILL path, killed=true); all
      seven executes take the signal (entry checks → "Operation aborted",
      bash → kill tree + "Command aborted" after partial output, edit/
      write check inside the mutation queue, grep/find kill the child);
      read applies the non-vision image note off `ctx.model`, which
      agent.lua now passes (`model = config.model`). Divergences the
      oracle caught, fixed: bash exit-code errors keep the "(no output)"
      default text; find has no "Path not found" pre-check (fd stderr
      passes through); OutputAccumulator getLastLineBytes tracks the true
      open-line size across tail trims; write's success message counts JS
      UTF-16 units. Boundary (recorded in tool_parity.rs): grep/find
      cases restricted to single-matching-file outputs (rg/fd parallel
      traversal ordering); image cases use a PNG within pi's resize
      limits where auto-resize returns original bytes — resize mechanism
      and blockImages are 5.3.

      **Evidence:** `cargo test -p pi-rs-app --test tool_parity` compares
      all 63 cases; negative control: reverting the bash "(no output)"
      fix fails at bash-exit-code with an error-string diff. `pi.exec`
      signal exercised by `examples/extensions/exec-demo.lua`
      (`exec-abort`) + os_bindings.rs (abort mid-run kills with partial
      output, pre-aborted kills immediately); agent-loop ctx.model pinned
      by `execute_ctx_carries_the_current_model`. `cargo test
      --workspace`: 525 passed; `scripts/ui-diff`: all 12 suites match.

- [x] **5.3 Images end-to-end.** Mechanism: the photon slice pi uses
      (`@silvia-odwyer/photon-node` 0.3.4) ported byte-for-byte over the
      same pinned `image` 0.24.9 stack (`crates/pi-rs-host/src/image.rs`:
      resizeImageInProcess with the PNG-vs-JPEG candidate loop, quality
      steps, 0.75 backoff; image-convert convertToPng; the full
      exif-orientation.ts JPEG/WebP TIFF parse + flip/rotate) as
      `pi.image.resize`/`pi.image.convert_to_png` (blocking-pool async —
      pi's worker thread); `pi.clipboard.read_image` ports
      clipboard-image.ts (wl-paste/xclip probing, format preference, WSL
      PowerShell fallback, spawnSync timeout/maxBuffer semantics,
      BMP→PNG conversion) with the spec's `{env, platform}` test seam,
      plus `pi.random_uuid`/`pi.fs.tmpdir`; `settings-manager.ts` port
      moved pi-rs-app→pi-rs-host (auth-storage precedent; re-exported) and
      bound per-VM as `pi.settings.block_images`/`set_block_images`.
      Policy: read.lua runs the exact read.ts resize branch
      (formatDimensionNote, omitted-image note, non-vision note order);
      utils/messages.lua ports sdk.ts convertToLlmWithBlockImages
      (per-request dynamic read, mapped-array dedupe) and both product
      packs pass it as convertToLlm; interactive.lua
      packs pass it as convertToLlm; interactive.lua
      handle_clipboard_image_paste is the full
      handleClipboardImagePaste (temp write `pi-clipboard-<uuid>.<ext>`;
      silent errors).
      Boundaries (recorded in code): the native clipboard addon
      (`@mariozechner/clipboard`, macOS/Windows/X11) is not ported — pi-rs
      behaves as pi does when `loadClipboardNative` cannot resolve the
      addon; `autoResizeImages` is an embedder-only option (coding agent
      always default-true).

      **Evidence:** `tests/image-parity/` oracle (21 cases — passthrough,
      dimension/byte-cap resizes, quality walk, dimension backoff,
      unresizable-null, EXIF 3/5/6/8 under resize, GIF/WebP/JPEG inputs,
      convert passthrough/EXIF/garbage-null) generated from the vendored
      photon-node WASM via `scripts/image-oracle` replays byte-identically
      through the public Lua surface (`cargo test -p pi-rs-host --test
      image_parity`); negative control: without jpeg-decoder's
      `platform_independent` (WASM scalar IDCT vs native SIMD) the
      jpeg-decode cases fail on encoded bytes. tool-parity gained
      `read-image-resized(-nonvision)` — pi's real read tool resizing a
      2200x8 PNG through Photon matches pi-rs's wired path including the
      "original 2200x8, displayed at 2000x7 … 1.10" note (65 cases).
      `block_images_replay.rs` pins the filter end-to-end: with
      `.pi/settings.json` blockImages, the second provider request
      carries "Read image file [image/png]\nImage reading is disabled."
      and no image block while the session toolResult keeps the image.
      `clipboard_bindings.rs` scripts wl-paste/xclip stubs (Termux
      short-circuit, bare-X11 nil, preference order with parameterized
      raw types, BMP→PNG, xclip fallback); `settings_bindings.rs` pins
      default/merge/granular-persist. New hooks exercised by
      `examples/extensions/image-demo.lua`, `clipboard-demo.lua`,
      `settings-demo.lua`. `cargo test --workspace`: 532 passed;
      `scripts/ui-diff`: all 12 suites match; `nix flake check` green.

- [x] **5.4 Agent-loop event-order oracle.** `tests/agent-parity/` oracle
      (23 cases) generated from pi's real `Agent`/agent-loop.ts
      (`scripts/agent-oracle` → gen-oracle.ts drives scripted streams,
      scripted tools, scripted hooks, and event-count triggers) replays
      through the public surface — `pi.agent.new` via
      tests/agent-parity/driver.lua, loaded like a user extension —
      comparing the full subscriber event sequence, per-stream-call
      request snapshots (model/reasoning/systemPrompt/converted
      messages), per-phase prompt/continue outcomes, and final agent
      state (timestamps scrubbed to 0 on both sides; Lua's empty-table
      `{}`/`[]` encoding artifact folded). Coverage: steering/follow-up
      drain modes (one-at-a-time and all, preseeded and mid-run
      triggers), continue() drain paths (skip-initial-steering-poll,
      from-user context, from-assistant error), parallel vs sequential
      ordering (completion-order ends vs source-order result messages,
      per-call message emission, per-tool `executionMode` override,
      immediate outcomes interleaving), settlement (unknown tool,
      validation errors, before-block/throw, after-merge/throw,
      terminate full/partial), and abort transitions (mid-stream,
      mid-sequential-batch, mid-parallel-prepare, stream error
      stopReason, streamFn throw → handleRunFailure events).
      Divergences the oracle caught, fixed: agent.lua's
      `has_more_tools = false` abort shortcut removed (pi streams one
      more aborted assistant turn after an aborted tool batch), parallel
      `tool_execution_end` now emitted per completion inside each task
      (was after the whole batch), sequential tool-result messages now
      emitted per call before the next tool starts (was batch-end), the
      `thinkingLevel == "off" and nil or …` Lua precedence bug that
      always sent `reasoning`, and schema.rs validation errors reworded
      and aggregated to typebox's en_US locale strings/order
      ("must have required properties x, y" addressed at the first
      missing property; "must be integer").

      **Evidence:** `cargo test -p pi-rs-agent --test agent_parity`
      compares all 23 cases. Negative controls: reintroducing the abort
      shortcut fails at abort-during-sequential-batch; moving parallel
      end emission back after `pi.parallel` fails at
      parallel-completion-order events[15]. No new public hooks — no
      example needed. `cargo test --workspace`: 533 passed;
      `scripts/ui-diff`: all 12 suites match.

      **Accept (whole item):** deterministic scenario replays yield
      equivalent provider messages, filesystem/process effects, event
      order, and terminal frames. ✓ via the 4.1 protocol oracle (29
      cases), 5.1 system-prompt oracle, 5.2 tool oracle (65 cases,
      effects byte-for-byte), 5.4 event-order oracle (23 cases), and
      the 4.2 provider-turn frames (14 checkpoints).

**6. Match sessions and continuity** — creation, naming, persistence,
resume, branching, compaction, context reconstruction, and session UI in
the order they become reachable from the interactive product. Split into
rungs:

- [x] **6.1 Session persistence through the product.** Mechanism:
      `pi.session.*` (crates/pi-rs-host/src/session.rs) binds pi-rs-session's
      `session-manager.ts` port as per-session userdata handles —
      `create`/`open`/`in_memory` constructors (default sessionDir/agentDir
      from cwd + discover), the append surface agent-session.ts and sdk.ts
      call (message, thinking_level_change, model_change, session_info,
      custom_entry, custom_message_entry), and the read side (file/id/name/
      cwd/leaf/header/entry/entries/branch, build_session_context,
      is_persisted); the old ad-hoc `pi.session_append` (reopened the file
      per call) is ablated. Policy: `utils/agent-session.lua` shared
      fragment — `persist_agent_event` (the _handleAgentEvent persistence
      slice: user/assistant/toolResult as message entries, custom as
      custom_message entries) and `session_startup_appends` (sdk.ts
      restore-vs-initial appends) — wired into both product packs:
      interactive `create_interactive_state` constructs the session and
      persists from the subscribe seam, `session_set_model`/cycle now
      append model_change (closing the 3a.3 deferral; settings persist and
      thinking re-clamp stay deferred to items 5/9 and 7), and pi-rs-run
      builds its own session (main.rs no longer pre-creates one — the
      manual user-message append and duplicate-open path died with it).
      Deferred, recorded in code: message restore + `--continue`/`--resume`
      (6.2), custom-message oracle coverage (extension surface, item 9).

      **Evidence:** `tests/session-parity/` oracle (12 cases) generated
      from pi's real `AgentSession` + `SessionManager` over scripted
      streams/tools/event-count triggers (`scripts/session-oracle` →
      gen-oracle.ts) replays through the product policy via the
      `session-parity` command; `cargo test -p pi-rs-app --test
      session_parity` compares the persisted JSONL entry-for-entry (uuids
      normalized by first appearance, timestamps scrubbed, cwd
      substituted; values compared parsed since Lua tables don't preserve
      JS key order). Coverage: file-creation deferral until the first
      assistant message (no-file cases), single/multi/thinking/tool/error/
      aborted turns, steering + follow-up queue entries, session_info,
      model_change mid-session. Negative controls: swapping the sdk.ts
      startup-append order fails at single-turn; dropping toolResult
      persistence fails at tool-turn. New hook exercised by
      `examples/extensions/session-demo.lua` + `pi-rs-host
      tests/session_bindings.rs` (deferred persist, entry sequence, leaf/
      context getters, reopen, in-memory). anthropic/block-images replays
      now run the product path end-to-end (session created inside
      pi-rs-run). `cargo test --workspace`: 535 passed; `scripts/ui-diff`:
      all 12 suites match.

- [x] **6.2 Resume and context reconstruction.** CLI: `--continue`/`-c`
      and `--session <path|id>` parse per args.ts;
      `cli/session_select.rs` ports main.ts `resolveSessionPath` +
      `createSessionManager` routing (direct path, local/global id-prefix
      match, global → fork-confirm prompt with pi's exact strings +
      `Aborted.` exit 0, not-found → error exit 1, continueRecent
      most-recent-or-create with the custom-dir cwd filter); main.rs
      re-homes the effective cwd to the session header
      (`sessionManager.getCwd()` semantics) and hands
      sessionFile/sessionDir/modelFromCli/thinkingFromCli/
      defaultThinkingLevel to the packs. Policy in
      `utils/agent-session.lua`: `construct_session` (pi.session.open/
      create) + `session_startup` — the sdk.ts restore slice: model via
      `pi.ai.find_model` + `has_configured_auth` with the "Could not
      restore model X. Using Y" fallback, thinking precedence
      (CLI > saved entry > settings default) + missing-entry backfill,
      `agent.state.messages` seeded from buildSessionContext, new-session
      initial appends — run by both product packs (pi-rs-run also gained
      the per-provider `getApiKey` seam, per sdk.ts streamFn).
      interactive.lua ports `renderInitialMessages`/`renderSessionContext`
      (user/assistant/toolResult rows, aborted/error tool settlement,
      populateHistory into the editor, compaction-count status) and
      start()'s modelFallbackMessage warning. Mechanism:
      `pi.session.open` passes the cwd override only when given (header
      cwd default). Deferred, recorded in code: `--resume` + the
      SessionSelectorComponent, missing-session-cwd prompt, and the
      shutdown "To resume this session" line move to 6.3; restored
      bashExecution/custom/summary/skill rows join 6.4/6.5 and items 7/9.

      **Evidence:** `tests/ui-parity/resume-turn.json` drives pi's real
      `SessionManager.open` + copied renderInitialMessages bodies over
      real components (`pi-resume-turn.ts`) against pi-rs's product wiring
      (create_interactive_state with `sessionFile`); `scripts/ui-diff`
      reports 6 resume-turn checkpoints (restored transcript incl. tool
      rows + "Operation aborted" settlement and footer usage/cache/
      context cells fed by the restored messages, ctrl+o expanded/
      collapsed, history up×2 recalling both restored user texts, resize)
      alongside the prior suites; negative control: skipping
      render_initial_messages fails at startup with a cursor diff.
      `resume_replay.rs` pins the next provider request carrying the
      rebuilt context exactly once ([hi, hello there, again]), the JSONL
      gaining only the new turn (no duplicate startup appends), thinking
      restore ("low"), catalog-model restore with configured auth over
      the CLI fallback, the exact fallback-message string, and the
      thinking-entry backfill. `session_select.rs` unit tests pin the
      resolveSessionPath/continueRecent semantics. No new public hooks
      (`pi.session.open` landed in 6.1) — no new example. `cargo test
      --workspace`: 542 passed; `scripts/ui-diff`: all 13 suites match.

- [x] **6.3 Session UI.** Mechanism: `pi.session.list`/`list_all` bind
      pi-rs-session's listing (SessionInfo rows; the spec's async progress
      callback is not bridged — listing is synchronous, so the transient
      "Loading…" header resolves within one dispatch), plus handle
      `uses_default_session_dir`; `pi.tui.fuzzy_match` and
      `pi.tui.js_regex_search` (the hljs JS→fancy-regex engine without the
      `m` flag) expose the selector-search seams. Mechanism divergences the
      fixture caught, fixed 1:1: `truncate_to_width` now ports
      `finalizeTruncatedResult` (full resets after the cut and the
      ellipsis; pending-ANSI dropped at the cut) and the Lua chalk styles
      port `stringEncaseCRLFWithFirstIndex` (newlines encased close/open).
      Policy in interactive.lua: session-selector-search.ts
      (`re:` regex / quoted-phrase / fuzzy tokens, recent/relevance sort),
      the full SessionSelectorComponent (3-line header with scope/name/sort
      column, threaded tree build+flatten with │└├ prefixes, maxVisible-10
      windowing, cwd/path columns, delete confirm → trash-then-unlink with
      current-session guard and timed status messages, ctrl+r rename panel,
      empty-list messages), `show_session_selector` →
      `handle_resume_session` with the missing-session-cwd confirm
      (core/session-cwd.ts + showExtensionConfirm) and cwd-override retry,
      `/new` (`handle_clear_command`), `/name`, `/session` stats
      (toLocaleString/toFixed ports), the footer's "• sessionName", and
      the shutdown "To resume this session:" line
      (quoteIfNeeded/formatResumeCommand). Runtime replacement: the sdk.ts
      construction slice extracted as `bind_session_runtime` — /new and
      /resume rebuild the agent over the new manager (restore slice, new
      cwd system prompt, resubscribe with stale-agent guard,
      renderCurrentSessionState) exactly like AgentSessionRuntime;
      session_before_switch/shutdown/start extension events join item 9
      (recorded in code). CLI: `--resume`/`-r` → the Lua
      `pi-rs-resume-picker` (selectSession's standalone selector TUI, "No
      session selected" exit 0), startup missing-cwd → headless
      MissingSessionCwdError exit 1 or the Lua `pi-rs-startup-selector`
      Continue/Cancel with `cwdOverride` reopen; fatal runtime errors exit
      1 through the pi-rs-interactive `exitCode`.

      **Evidence:** `tests/ui-parity/session-turn.json` drives pi's real
      `SessionSelectorComponent`/`ExtensionSelectorComponent` + copied
      interactive-mode bodies over a fixed session-dir tree with a pinned
      `Date` and PATH="" (`pi-session-turn.ts`) against pi-rs's product
      wiring (`interactive-session-parity-sequence`); `scripts/ui-diff`
      reports 26 session-turn checkpoints (selector open/navigate, fuzzy +
      `re:` filters, scope toggle with cwd column, path/sort/named
      toggles, delete confirm/cancel/done + current-session error, rename
      panel/typed/saved, resume with footer session name, /session info
      block, /name set/show, missing-cwd confirm → resumed-in-current-cwd,
      /new, resize) alongside the prior suites (6+6+21+19+13+14+6+13+5+6+
      7+24+15); scenarios pin `fixedCwd` because /session and the cwd
      prompt surface absolute paths. Negative control: perturbing the tree
      branch prefix fails at resume-open cell (24,5).
      `interactive_session.rs` pins the runtime switch: after `/resume`
      the next provider request carries the selected session's context
      exactly once and persists into that file only; after `/new` the
      request carries no prior context and lands in a fresh file. New
      hooks exercised by `examples/extensions/session-demo.lua` (list/
      list_all) + `fuzzy-demo.lua` (fuzzy_match/js_regex_search) with
      pi-rs-host tests. The picker and startup-selector TTY paths reuse the
      pinned components; their process wiring is exercised manually
      (`pi --resume`). `cargo test --workspace`: 544 passed.

- [x] **6.4 Branching and tree navigation.** Mechanism: `pi.session`
      gained the branching surface (`get_tree` with labels/timestamps
      resolved onto nodes, `branch`, `reset_leaf`, `branch_with_summary`,
      `create_branched_session`, `new_session`, `append_label_change`,
      `parse_iso_ms`); `pi.settings` the tree reads (`double_escape_action`,
      `tree_filter_mode`, `branch_summary`); `pi.ai.stream_simple` maps
      `maxTokens`; `pi.now_ms` (JS `Date.now()` — the spec's double-escape
      gate compares epoch ms, and the second-granular `os.time()` fallback
      missed presses across a second boundary); `binding_matches` ports the
      keys.ts legacy shift+letter→uppercase match (shift+l/shift+t). Policy
      in interactive.lua + `utils/branch-summary.lua`: tree-selector.ts 1:1
      (flatten/refilter with active-branch-first ordering, connectors +
      positioned gutters, fold ⊞/⊟, five filter modes + cycling, token
      search, label editing/timestamps, segment jumps),
      user-message-selector.ts, branch-summary-message.ts (+ restored
      branchSummary transcript rows), extension-editor.ts, navigateTree +
      getUserMessagesForForking (agent-session.ts) over the
      branch-summarization.ts + compaction/utils.ts ports (token-budgeted
      prepare, file-ops tracking, `serializeConversation` over the 2b.5
      wire-key order, the exact prompts), AgentSessionRuntime.fork
      (before/at, fork-to-root, runtime rebuild), showTreeSelector's
      summarize-choice loop + summarizing Loader + escape-abort override,
      showUserMessageSelector, handleCloneCommand, `/tree` `/fork` `/clone`
      routing, and the 3.3 double-escape targets. Divergence fixed: the
      footer sums usage over ALL session entries (abandoned branches
      count), not the context path. Deferred, recorded in code:
      session_before_tree/session_tree/session_before_fork extension
      events (item 9), flushCompactionQueue after navigation (6.5).

      **Evidence:** `tests/ui-parity/tree-turn.json` drives pi's real
      TreeSelector/UserMessageSelector/BranchSummary/ExtensionSelector/
      ExtensionEditor components + real generateBranchSummary against a
      held stub with the copied interactive-mode bodies
      (`pi-tree-turn.ts`) vs pi-rs's product wiring
      (`interactive-tree-parity-sequence`, per-keystroke renders so the
      hidden hardware cursor rests where pi's differential writes leave
      it); `scripts/ui-diff` reports 40 tree-turn checkpoints (restored
      [branch] row collapsed/expanded, tree open/nav/fold/unfold, all
      filters, search + clear, label prefill/set/timestamps, summarize
      selector escape-loop + custom-prompt editor, summarize loader →
      navigated-with-summary, abort loader → cancelled + re-shown tree,
      fork selector/nav/fork, clone, double-escape tree, resize —
      including footer usage cells over both branches) alongside the
      prior suites; negative control: a perturbed gutter glyph fails at
      tree-open (18,2). `interactive_tree.rs` pins the summarization
      request byte-exactly (system prompt, serialized conversation +
      default prompt, max_tokens 2048), the branch_summary entry at the
      navigation target (preamble + text, fromHook false, editor
      prefill, leaf), escape-abort (no entries appended, tree re-shown),
      fork (root→parent copy, parentSession header, off-path labels
      dropped, source untouched), and clone (path-at-leaf copy). New
      hooks exercised by `session-demo.lua` + `settings-demo.lua` and
      their pi-rs-host tests. `cargo test --workspace`: 548 passed;
      `scripts/ui-diff`: all 15 suites match; `nix flake check` green.

- [x] **6.5 Compaction.** Policy in `utils/compaction.lua` (shared
      fragment, both packs): compaction.ts ported 1:1 — token estimation
      over usage (estimate/calculate/shouldCompact), findCutPoint/
      findTurnStartIndex (toolResult never cut, non-message backscan,
      split turns), prepareCompaction (previous-summary merge, fromHook
      gate, file-ops extraction), generateSummary/generateTurnPrefixSummary
      (initial/UPDATE/turn-prefix prompts, custom instructions,
      0.8/0.5-reserve maxTokens with model clamp, reasoning passthrough),
      compact (Promise.all as pi.parallel, file-list appendix + details) —
      plus pi-ai overflow.ts isContextOverflow over the
      pi.tui.js_regex_search mechanism. The agent-session.ts slice lives
      in interactive.lua's per-runtime compaction slice: manual compact()
      (Already compacted / Nothing to compact / cancelled semantics),
      _checkCompaction (compaction-boundary timestamp gate, overflow
      compact-and-retry-once latch, error-message estimation gate),
      _runAutoCompaction (willRetry error-strip, queued-message continue),
      _handlePostAgentRun + the pre-prompt check wired into
      session.prompt. UI: /compact routing + handleCompactCommand,
      compaction_start/end handlers (loader labels, escape-override swap,
      chat rebuild + CompactionSummaryMessageComponent port), compaction
      queue (queue during compaction/branch-summary, pending rows,
      unconditional flush incl. after tree navigation — closing the 6.4
      deferral), restored compactionSummary rows. Mechanism:
      `append_compaction` + standalone `pi.session.build_context`,
      `pi.settings.compaction_settings`, stream_simple `reasoning`, and
      the `__pi_rs_json_array` metatable flag (decoded arrays round-trip;
      empty details file lists persist as `[]` like pi). Footer's
      duplicated token-estimation slice ablated in favor of the fragment.
      Deferred, recorded in code: session_before_compact/session_compact
      extension events (item 9), extension commands in the compaction
      queue (item 9), retry-loop interplay (items 4/5/7), pi-rs-run's
      post-run compaction (item 10).

      **Evidence:** `tests/compaction-parity/` oracle (37 cases: 10
      prepare shapes, 8 compact request/summary shapes incl. split-turn
      budgets and errors, token/should cases, 12-case overflow battery)
      generated from pi's real compaction.ts + overflow.ts
      (`scripts/compaction-oracle`, injected streamFn recording requests)
      replays through the Lua fragment via the `compaction-parity`
      command (`cargo test -p pi-rs-app --test compaction_parity`);
      negative control: disabling the cut-point backscan fails at
      prepare-nonmessage-backscan. `tests/ui-parity/compaction-turn.json`
      drives pi's real Agent + streamAnthropic + real prepareCompaction/
      compact/CompactionSummaryMessageComponent over a restored session
      with the copied agent-session/interactive-mode bodies
      (`pi-compaction-turn.ts`) against pi-rs's product wiring —
      `scripts/ui-diff` reports 13 compaction-turn checkpoints (restored
      [compaction] row + "Session compacted 1 time", expanded
      "Compacted from 1,234 tokens" markdown, threshold auto-compaction
      loader + rebuilt chat at "Compacted from 190,004 tokens",
      "Compaction failed: Already compacted", manual loader, UPDATE-path
      compact-done, queue-during-compaction status + steering row,
      escape-cancel + unconditional flush running the queued prompt,
      resize) alongside the prior suites; negative control: a perturbed
      auto-loader label fails at auto-loader (23,20).
      `interactive_compaction.rs` pins behavior end-to-end against a
      scripted stub: /compact's exact summarization request (system
      prompt, serialized conversation, model-clamped maxTokens 1024) +
      JSONL compaction entry (firstKeptEntryId, tokensBefore 170,
      fromHook false, `[]` details) + next request carrying the compacted
      context exactly once; escape-cancel appends nothing and sends no
      request; queued messages flush into the next prompt; big-usage
      threshold auto-compaction; overflow compact-and-retry with the
      error stripped from the retry context. New hooks exercised by
      `examples/extensions/session-demo.lua` + `settings-demo.lua` and
      their pi-rs-host tests. `cargo test --workspace`: 554 passed;
      `scripts/ui-diff`: all 16 suites match; `nix flake check` green.

      **Accept (whole item):** Pi-compatible fixtures reopen to equivalent
      state and the next provider request contains equivalent context
      exactly once. ✓ via 6.1 persistence oracle (12 cases), 6.2
      resume_replay (context exactly once), 6.3/6.4 runtime-switch and
      tree pins, and 6.5's compaction oracle + provider replays
      (compacted context exactly once, threshold/overflow auto paths).

## Then — close the coding-agent surface

**7. Port interactive states and commands** — inventory of the pinned
coding agent's remaining reachable surface (2026-07-10, vs the routed
set): `!`/`!!` bash mode; thinking levels; `/settings` (+ theme /
thinking / show-images / config selectors), `/scoped-models`, `/trust`,
`/export` `/import` `/share` `/copy`, `/changelog` `/hotkeys` `/debug`
`/reload` + easter eggs and the update notification; the provider-retry
surface; external editor + suspend. Split into rungs:

- [x] **7.1 Bash mode (`!`/`!!`).** Policy: `utils/bash-executor.lua`
      (core/bash-executor.ts executeBashWithOperations + the
      createLocalBashOperations/getShellConfig/sanitizeBinaryOutput
      slices over pi.exec, rolling buffer in JS string-length units,
      `pi-bash-*.log` temp persistence); interactive.lua ports
      bash-execution.ts (constructor-vs-updateDisplay content, collapsed
      preview over truncateToVisualLines, hidden-lines/cancelled/exit/
      truncation status rows, the spec's updateDisplay header that turns
      excluded commands green), handleBashCommand (deferred-to-pending
      during streaming, per-chunk render), flushPendingBashComponents +
      updatePendingMessagesDisplay clear semantics at every queue-change
      site, the `!`/`!!` submit branch (running → warning + text
      restore), escape's isBashRunning/isBashMode branches, restored
      bashExecution transcript rows, and isBashMode as text-derived
      state with the spec's post-settle false override (bug-for-bug: a
      warning-restored `!…` editor keeps the thinking border until
      typing resumes — escape then does NOT clear). Session half in
      bind_session_runtime: executeBash (settings prefix/shellPath,
      sync-armed abort signal), recordBashResult, abortBash,
      _pendingBashMessages flushed at _runAgentPrompt/pre-prompt-
      continue finallys and before new prompts. Mechanism:
      `pi.settings.shell_command_prefix`/`shell_path`; tools-pack
      truncate/visual-truncate exports shared cross-pack; ui-diff
      gained `UI_DIFF_DUMP=<checkpoint>` row dumps. Deferred, recorded
      in code: the user_bash extension event (item 9).

      **Evidence:** `tests/ui-parity/bash-turn.json` runs real
      subprocesses through pi's real BashExecutionComponent +
      executeBashWithOperations with the copied interactive-mode bodies
      (`pi-bash-turn.ts`, real Agent + SSE stub for the streaming
      section) and pi-rs's product machinery
      (`interactive-bash-parity-sequence` over
      create_interactive_state); `scripts/ui-diff` reports 18 bash-turn
      checkpoints (bash-mode border on typing, escape clear, completed
      echo, seq collapsed “... 6 more lines”/expanded/recollapsed, exit
      3, `!!` dim borders + green header + the shrink scroll artifact
      (chunk-time renders land on both sides; `; sleep 0.3` keeps pi's
      render tick deterministic), running loader, already-running
      warning, cancel keeping restored text under the override's gray
      border, ctrl+c clear, mid-stream deferred pending row, abort
      clearing the pending display, flush-into-chat turn, resize);
      oracle regenerated twice byte-identically. Negative control:
      rendering the updateDisplay header with colorKey instead of the
      spec's hardcoded bashMode fails at bash-excluded (14,1).
      `interactive_bash.rs` pins JSONL persistence (entry shape,
      excludeFromContext), the next request carrying
      bashExecutionToText exactly once, `!!` never reaching the
      provider, and deferred flush ordering after an aborted turn
      (documenting pi's transformMessages aborted-assistant skip);
      `submit_router_matches_pi_bash_interception` pins routing.
      New settings getters exercised by
      `examples/extensions/settings-demo.lua` + settings_bindings.
      `cargo test --workspace`: 558 passed; `scripts/ui-diff`: all 17
      suites match.

- [x] **7.2 Thinking levels.** Policy in interactive.lua: the
      agent-session.ts thinking slice (setThinkingLevel's clamp +
      change-gated session/settings persistence, cycleThinkingLevel,
      getAvailableThinkingLevels, supportsThinking,
      _getThinkingLevelForModelSwitch) with shift+tab wired as
      app.thinking.cycle in setup_shell_editor and the interactive-mode.ts
      status rows; session_set_model and session_cycle_model now re-clamp
      (scoped explicit levels override) — closing the 3a.3 and 6.2
      deferrals; utils/agent-session.lua session_startup defaults to
      DEFAULT_THINKING_LEVEL "medium", clamps to the resolved model, and
      reads the settings default from `pi.settings` directly (the main.rs
      plumbing died). Mechanism: `pi.ai.supported_thinking_levels` /
      `clamp_thinking_level` — duck-typed over `reasoning` +
      `thinkingLevelMap` like the JS original (pi-rs-ai-types algorithm
      split into `*_for` seams, still written once), reconstructing the
      map's explicit-null "unsupported" markers from convert.rs's
      decode-order metatable (JSON null decodes to an absent Lua key; 120
      catalog rows carry such entries) — and
      `pi.settings.default_thinking_level`/`set_default_thinking_level`.
      Deferred, recorded in code: pi's ThinkingSelectorComponent is
      exported but unreachable in the coding-agent product (the `/settings`
      thinking submenu lands in 7.3); settings thinkingBudgets/
      maxRetryDelayMs agent options join item 9; app.thinking.toggle stays
      7.10 (`hideThinkingBlock` persistence lands in 7.3).

      **Evidence:** `tests/ui-parity/thinking-turn.json` drives pi's real
      CustomEditor + FooterComponent + real getSupportedThinkingLevels/
      clampThinkingLevel with the copied agent-session/interactive-mode
      bodies (`pi-thinking-turn.ts`) against pi-rs's product wiring
      (`interactive-thinking-parity-sequence`); `scripts/ui-diff` reports
      12 thinking-turn checkpoints (shift+tab walk off→low→medium→high→
      xhigh→off skipping the null-mapped minimal, per-level border
      colors, footer "• level"/"• thinking off" suffixes, keep-level
      switch, re-clamp-to-off switch dropping the suffix, "Current model
      does not support thinking", settings-default restore after a
      non-reasoning detour, resize). `pi-model-turn.ts` gained the real
      setModel/cycleModel re-clamp bodies its stub lacked and model-turn
      was regenerated — cycle-forward now pins "Switched to GPT-5.4
      (thinking: medium)" (15 checkpoints; all 18 suites match). Negative
      control: disabling the null-map reconstruction fails thinking-turn
      at cycle-low ("minimal" vs "low"). `interactive_thinking.rs` pins
      behavior end-to-end through the product path: a new session
      defaults to medium (JSONL thinking_level_change "medium", anthropic
      `thinking {enabled, budget_tokens 8192}`), and cycling appends
      "low" (skipping null minimal), persists settings
      `defaultThinkingLevel`, and flips the next request from
      `{disabled}` to budget 2048. New hooks exercised by
      `examples/extensions/model-registry-demo.lua` + `settings-demo.lua`
      and their pi-rs-host tests. `cargo test --workspace`: 560 passed.

- [x] **7.3 `/settings` family.** `SettingsSelectorComponent` ported as Lua
      policy over the corrected `pi-rs-tui` SettingsList/SelectList mechanisms:
      exact search, centered max-10 window, aligned values, descriptions,
      hints/borders, every value choice, warnings submenu, thinking submenu,
      and dark/light theme submenu with Pi's live preview, cancel restore,
      commit, and mixed construction-time/dynamic style behavior. `/settings`
      routes through the editor-slot swap; all callbacks persist through the
      expanded `pi.settings` bridge and apply live where observable (queue
      modes + transport, thinking, hidden thinking, editor padding/menu size,
      hardware cursor, clear-on-shrink, terminal progress). Model select/cycle
      now call `setDefaultModelAndProvider`, closing 3a.3. The process control
      seam carries mutable cursor/shrink/progress preferences without moving
      policy into Rust.

      **Scope correction:** `theme-selector.ts` and
      `show-images-selector.ts` are exported compatibility components but have
      no coding-agent product path in the pinned snapshot; `config-selector.ts`
      belongs to `package-manager-cli.ts` (`pi config`), not `/settings`.
      Their compatibility/config-package surface is therefore inventoried in
      item 9 rather than counted as reachable item-7 UI.

      **Evidence:** `tests/ui-parity/settings-turn.json` drives Pi's real
      `SettingsSelectorComponent` through the copied showSelector/submit
      wiring (`pi-settings-turn.ts`) against pi-rs's product route
      (`interactive-settings-parity-sequence`); `scripts/ui-diff` matches 15
      checkpoints (open/search/windowing, auto-compact cycle, thinking +
      warning submenus, dark→light preview/cancel/commit, restored editor,
      resize) alongside all prior suites. Negative control: static submenu
      styles fail at `theme-preview-light`. `settings-demo.lua` +
      `settings_bindings.rs` exercise representative scalar/nested settings
      and pin theme, queue modes, timeout, editor preferences, warnings, and
      default provider/model to `settings.json`. `cargo test --workspace`:
      560 passed; `cargo fmt --check` green.

- [x] **7.4 `/scoped-models`.** `ScopedModelsSelectorComponent` ported as
      embedded Lua policy: DynamicBorder/header/search/list/footer composition,
      centered max-8 window, fuzzy filter, wrap navigation, implicit-all state,
      toggle/filtered enable+clear/provider toggle, Alt+Up/Down ordered moves
      (including the implicit-all no-op), dirty/unsaved footer, Ctrl+S persist,
      Ctrl+C search clear/cancel, and editor-slot restore. The
      interactive-mode `showModelsSelector` wiring now applies changes to the
      live session scope, updates provider counts, persists canonical ordered
      IDs through `pi.settings.enabled_models/set_enabled_models`, and restores
      selector-persisted scope on startup; Ctrl+P cycling consumes that order.
      Broader user-authored `enabledModels` glob/thinking pattern compatibility
      remains with item 9's configuration surface (the selector itself persists
      canonical IDs).

      **Evidence:** `tests/ui-parity/scoped-models-turn.json` drives Pi's real
      component + copied interactive-mode wiring (`pi-scoped-models-turn.ts`)
      against pi-rs's product policy (`interactive-scoped-models-parity-sequence`);
      `scripts/ui-diff` matches 14 checkpoints (open/all-state reorder no-op,
      toggles, ordered move, fuzzy filter, filtered clear, search clear,
      provider enable, save, cancel/editor restore, ordered cycle, resize)
      alongside all prior suites. `interactive_scoped_models.rs` pins the live
      order `[gpt-5.2, gpt-5.4, gpt-5-mini]`, next-cycle result, and persisted
      `settings.json`; `settings-demo.lua` + settings bindings exercise the new
      public store seam. `cargo test --workspace`: 561 passed; `cargo fmt
      --check` green.

- [x] **7.5 Automated model catalog freshness.** Replace the one-off Bun
      converter with a Nix-exposed update task that can ingest the current
      upstream model catalog, normalize it into `crates/pi-rs-ai/data/models.json`,
      and preserve pi-rs's catalog-as-data boundary. During parity, upstream Pi's
      generated catalog remains the behavioral source of truth; source-specific
      fetching/parsing stays update tooling, never runtime registry policy.
      Normal builds/checks remain offline and consume the reviewed, checked-in
      snapshot.

      Add scheduled automation that detects upstream catalog changes and opens a
      reviewable generated PR rather than mutating installed pi-rs catalogs at
      runtime. The PR records source revision/hash and provider/model count diffs;
      generation rejects unknown schema, duplicate provider/model IDs, unsupported
      wire protocols, and rows that do not round-trip through `Model`. Catalog,
      protocol replay, and flake checks gate merge. Document the manual command,
      override mechanism for upstream metadata defects, and the deliberate
      promotion path for a newly required protocol/provider.

      **Accept:** `nix run .#update-model-catalog` is idempotent for a fixed input
      and regenerates the embedded snapshot; a fixture-backed/offline test pins
      normalization; scheduled CI produces no diff when current and a generated
      PR with provenance + inventory summary when stale; merge checks prove the
      result conforms to the reviewed `Model` schema and protocol vocabulary.
      End-to-end dispatch coverage for every advertised API remains item 8's
      protocol-replay acceptance gate.

      **Landed slice (2026-07-11):** `nix run .#update-model-catalog` now owns
      source acquisition (latest ref, exact revision, or local checkout), strict
      normalization, reviewed `model-catalog-overrides.json`, deterministic
      provenance/output hashes + inventory/PR summary, and the checked-in
      snapshot; the one-off `gen-models-json.ts` is gone. The offline
      `tests/model-catalog-update/` exerciser proves fixed-input idempotency,
      order/override behavior, and fail-closed unknown-field/protocol handling;
      `registry.rs` pins provenance inventory, the reviewed API vocabulary, and
      every-row `Model` round-trip. `.github/workflows/model-catalog-update.yml`
      runs generation + `nix flake check` weekly and creates/refreshes the
      generated branch/PR only for a valid diff. Manual and protocol-promotion
      paths are documented in README.

      **Closure (2026-07-11):** the original dispatch wording was assigned to
      the wrong rung. Catalog update tooling can validate a reviewed API
      vocabulary but cannot prove each transport implementation; that requires
      item 8's deterministic protocol replays. Keeping 7.5 blocked on item 8
      would violate the ladder's ordering without adding freshness coverage, so
      the dispatch requirement now lives solely in item 8. During parity the
      checked-in snapshot remains pinned to Pi v0.79.0; adopting later catalog
      schema or behavior is a deliberate spec promotion, not an automated
      metadata refresh.

      **Evidence:** updater fixture green; pinned remote revision regenerates
      `models.json` byte-identically; focused registry test 11/11; `cargo fmt
      --check` + `nix fmt -- --check flake.nix` green;
      `nix build .#checks.x86_64-linux.model-catalog-update` green; `nix build
      .#checks.x86_64-linux.workspace-test --print-build-logs` green (562
      passed). Live automation is now pinned too: workflow run
      [29159252628](https://github.com/y0usaf/pi-rs/actions/runs/29159252628)
      proved the fixed-revision no-diff branch and full `nix flake check`; a
      schema-compatible stale revision created/refreshed the provenance-rich
      generated [PR #1](https://github.com/y0usaf/pi-rs/pull/1) (+10 models),
      then the flake merge gate rejected its deliberately stale 969-model count
      pin. The first stale run exposed `grep -q` closing the bare-boot CLI's
      stdout; `cbc65ea` made those checks pipe-safe before the successful branch
      refresh. Repository Actions permission was enabled for generated PRs.

- [x] **7.6 `/trust`.** Mechanism: the existing trust-manager/project-trust
      port is now exposed per VM as `pi.trust` (get/get_entry/set/set_many,
      option generation, path/input/config-dir discovery, prompt text), and
      `HostConfig.project_trusted` gates the VM's project settings manager.
      Main's startup path now resolves `--approve`/`--no-approve`, no-input,
      nearest saved decision, global default, interactive prompt, and headless
      fallback in Pi's order; the resolved decision gates both Rust and Lua
      settings and reaches the frontend. Policy in interactive.lua: the full
      TrustSelectorComponent port (saved direct/inherited labels + checkmark,
      bounded arrows/j/k, save/cancel), `/trust` editor-slot route, exact status
      row, and the untrusted-project startup warning. The startup ask reuses the
      Lua-authored ExtensionSelector pre-runtime TUI with Pi's five generated
      choices. Global extension `project_trust` interception remains item 9's
      resource-loader/configuration wiring; its event/store mechanisms and
      decision-order oracle already exist in discovery_trust.rs.

      **Evidence:** `tests/ui-parity/trust-turn.json` drives Pi's real
      TrustSelectorComponent + warning/status composition
      (`pi-trust-turn.ts`) against pi-rs's product policy
      (`interactive-trust-parity-sequence`); `scripts/ui-diff` matches 10
      checkpoints (untrusted startup warning, open, parent selection/save,
      inherited reopen, direct untrusted selection/save/reopen, cancel/editor
      restore, resize) alongside all 20 prior suites. `interactive_trust.rs`
      pins parent+child trust.json persistence through the product route;
      CLI parser tests pin both override spellings; the pre-existing 15-case
      discovery/trust suite pins prompt options, nearest inheritance, resolver
      order, extension answers, and untrusted discovery. New public hooks are
      exercised by `examples/extensions/trust-store-demo.lua` +
      `trust_bindings.rs`. `cargo test --workspace`: green; `cargo fmt --check`
      green; `nix build .#checks.x86_64-linux.workspace-test
      --print-build-logs`: green.

- [x] **7.7 `/login` provider breadth (pulled forward from item 8).** The OAuth
      registry now matches the pinned coding agent's three subscription providers:
      Anthropic, GitHub Copilot, and OpenAI Codex. Shared mechanism:
      `device_code.rs` ports RFC 8628 polling (immediate poll, default/minimum
      interval, permanent `slow_down` increments, timeout diagnostics, cancellation),
      and the host bridges the login dialog's cancellation signal + catalog model
      IDs without introducing an auth→catalog dependency. Codex ports browser PKCE
      and headless device-code selection, callback bind fallback, form exchanges,
      refresh, JWT `accountId`, and exact credential shape. Copilot ports enterprise
      domain normalization, device flow, short-lived token refresh, post-login model
      policy enables, dynamic API base URL from `proxy-ep`, and `enterpriseUrl`
      persistence. `/login`, `/logout`, `--login`, auth refresh, model-registry
      modification, and the existing Lua dialogs consume the providers through the
      same public registry/handle seams as Anthropic.

      **Boundary:** this closes built-in subscription authentication, not item 8's
      remaining transport breadth. Codex models still require the
      `openai-codex-responses` protocol, and Copilot's `openai-responses` rows require
      that protocol; those protocol replays remain item 8.

      **Evidence:** `subscription_providers.rs` replays Codex browser PKCE/manual
      exchange + device select/poll/exchange/JWT extraction and Copilot prompt →
      device poll → token refresh against captured loopback HTTP requests; registry
      tests pin exact built-in IDs/order, names, callback-server flags, refresh, and
      reset behavior. `auth_bindings.rs`
      pins all three through `pi.auth.oauth_providers`; `login-turn` now renders the
      three Pi-derived provider rows and matches at 24 checkpoints. Focused auth,
      host, and interactive-login tests + `cargo fmt --check` are green;
      `cargo test --workspace` and `nix build
      .#checks.x86_64-linux.workspace-test --print-build-logs` are green.

- [ ] **7.8 Session transfer commands.** `/export` (HTML/JSONL),
      `/import`, `/share`, `/copy`.

- [ ] **7.9 Info commands and chrome.** `/changelog`, `/hotkeys`,
      `/debug`, `/reload`, the easter eggs (armin/daxnuts/earendil),
      and the version-update notification.

- [ ] **7.10 Provider-retry surface.** _isRetryableError/_prepareRetry,
      retry loader + countdown-timer, retryEscapeHandler — closes the
      4/5/6.5 retry deferrals.

- [ ] **7.11 Remaining shell actions.** openExternalEditor
      (app.editor.external), ctrl+z suspend, thinking-block visibility
      toggle (app.thinking.toggle + hideThinkingBlock).

      **Accept (whole item):** every reachable interactive component has
      a parity fixture and no placeholder component or pi-rs-only chrome
      remains.

- [ ] **8. Complete coding-agent AI/auth compatibility.** Port the providers
      and model catalog behavior the pinned coding agent exposes, sharing
      transport/auth machinery rather than cloning provider implementations.
      Built-in `/login` breadth is complete via 7.7; this rung now concentrates on
      the remaining protocol families and provider-specific request behavior.

      **Accept:** the supported model inventory matches Pi's coding agent, every
      advertised API has deterministic protocol replays, and the three subscription
      providers' auth-state/request tests remain green.

- [ ] **9. Match coding-agent configuration and extensions.** Support the
      relevant Pi settings, themes, prompts, skills, packages, and extension
      use cases through pi-rs's Lua surface without changing their visible
      outcome. Translated Pi examples land in `examples/` as the conformance
      suite.
      Includes the package-manager `pi config`/ConfigSelector surface,
      custom/registered theme loading + watching, and exported compatibility
      components (`ThemeSelectorComponent`, `ShowImagesSelectorComponent`,
      `ThinkingSelectorComponent`) that are not reachable from interactive mode.

      **Accept:** translated reference fixtures exercise every exposed hook and
      representative Pi configurations produce equivalent behavior and frames.

- [ ] **10. Match non-interactive coding-agent modes.** Port the pinned CLI's
      print/JSON/RPC/export and other coding-agent modes that are part of the
      product, after the default interactive path is exact.

      **Accept:** argument, stdout/stderr, exit-status, and serialized-output
      differential tests pass against Pi.

- [ ] **11. Final parity audit.** Diff the complete public and reachable
      surface of `ref/pi/packages/coding-agent` and its required `ai`, `agent`,
      and `tui` behavior. Resolve or explicitly lock every difference.

      **Accept:** automated parity suites pass and side-by-side scripted
      sessions are visually indistinguishable. No unapproved product-visible
      divergence remains. Tag this baseline for compatibility maintenance and
      downstream forks.

## Post-parity — maintenance

Keep pi-rs compatible with Pi and deliberately port selected upstream fixes.
Custom agent loops, first-party packs, and product-specific behavior belong in
separate downstream forks, not this repository.

## Working rules

- Pi is the oracle. If a pi-rs test conflicts with Pi, update the implementation
  and the test; do not redefine the goal.
- Product behavior is Lua through the public surface — no Rust shortcutting. A
  placement that puts behavior in Rust needs a DESIGN locked-decision row.
- Every UI change includes a Pi-derived frame or input fixture. "Looks close"
  is not review evidence.
- No temporary UI, approximate component, knowingly different default, or
  pi-rs-specific label may satisfy a milestone.
- New public extension hooks should land with an `examples/` exerciser unless
  the commit explains why not.
- Ablation is sanctioned: delete code, docs, tests, and fixtures that no longer
  fit the boundary or that a port supersedes. Git is the attic.
- Implement internals idiomatically in Rust, judged by observable Pi behavior
  rather than source-layout similarity.
- Keep work inside the coding-agent boundary; no Discord bot, unrelated
  applications, or framework abstractions ahead of parity.
- Run focused tests and `cargo fmt --check` while iterating; run
  `cargo test --workspace` before closing an item and Nix checks for releases.
- Update the checkbox and cite acceptance evidence in the same change that
  completes an item.
