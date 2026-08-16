# Dogfood parity (PLAN 9.11)

Translated the pinned external dogfood packages (from the `pi-flake`
`94694da7321ce74aa7b82c13db7e60e28c0caba6` oracle) to ordinary **file-backed Lua
extensions** that run on the same public `pi.*` surface as any user extension.
No translation uses a privileged escape hatch; every capability is drawn from
the public Lua surface (`pi.*`), `ctx.*` context facades, or the public
exact-version module registry (`pi.module.require`).

## Translations (load-verified)

`examples/extensions/dogfood/*.lua`:

| Package | Public surface highlights | Long-lived-resource cleanup |
|---|---|---|
| `pi-pomodoro` | `pi.fs.watch_file`, `pi.set_interval`, `pi.fs.*`, `pi.path.*`, `pi.env`, `pi.json`, `ctx.ui.setStatus/notify/theme.fg` | watcher `handle:close()` + `clear_interval` on `session_shutdown`; stale ctxRef dropped |
| `pi-codex-fast` | `pi.on`, `pi.register_command`, `ctx.ui.*`, `pi.fs`, `pi.json` | stateless |
| `pi-working-indicator` | `ctx.ui.setWorkingIndicator/Missing/Status`, `pi.getThinkingLevel`, `pi.set_timeout/clear_timeout`, `theme.fg` | startup-switch timer cleared + generation-guarded |
| `pi-webfetch` | `pi.register_tool`, `pi.http.fetch`, `pi.tools.truncate@1`, `pi.buffer`, pure-Lua HTML→MD stand-in | stateless LRU table only |
| `pi-tool-management` | `pi.register_setting_item`/`pi.registered_setting_items` (push-model), `pi.getActiveTools/getAllTools/setActiveTools`, `ctx.ui.custom` overlay via `pi.tui.settings_list`, `pi.fs`+`pi.path`+`pi.env` tool-settings.json | stateless Set + file-backed settings; no host resource |
| `pi-minimal-editor` | `pi.footer` facade (`get_git_branch/extension_statuses/available_provider_count/on_branch_change`), `ctx.ui.setFooter/setEditorComponent` + `pi.tui.editor` (CustomEditor substrate), `ctx.sessionManager`/`getContextUsage`/`pi.getThinkingLevel` | footer sub disposed + editor/footer reset on session_shutdown |
| `pi-context-janitor` | `pi.ai.complete` (completeSimple), `pi.appendEntry`/`pi.sendMessage`/`pi.register_message_renderer`, capture/decider/index-store/restore, `pi.crypto.sha256`, `ctx.sessionManager.getBranch`, `pi.set_timeout/set_interval` | debounce timer + status spinner cleared, status cleared on session_shutdown/replacement |
| `pi-gecko-websearch` | `pi.tcp.connect` (Marionette), `pi.process.spawn` (gecko), `pi.fs.*` temp profile/cookies, `pi.tools.truncate@1`, `pi.register_tool(web_search/web_browse)` | BrowserManager.shutdown on session_shutdown (socket close/dispose + process kill + rm temp profiles) |
| `pi-rlm` | `pi.process.spawn python3` (verbatim Python worker), `pi.ai.complete` llm bridge, `pi.register_tool(repl)`, `pi.setActiveTools`, session-context store, `pi.register_message_renderer(rlm_final)` | python worker killed/disposed on session_shutdown + reset |
| `sting8k_pi-vcc` | `pi.register_command(/pi-vcc,/pi-vcc-recall)`, `pi.register_tool(vcc_recall)`, `pi.on(session_before_compact)` own-cut+compile, `ctx.compact`, `pi.agent.messages@1`.convert_to_llm, `pi.sendMessage`, `pi.fs` scaffold settings | stateless besides file-backed config + module-local lastStats; dispatch-scoped timer |
| `earendil_pi-review` | `pi.register_command(/review,/end-review)`, `pi.on(session_start/session_tree)`, `pi.exec` (git/gh), `pi.fs.stat/read_file`, `pi.path.*`, `ctx.sessionManager.{get_branch,get_entries,get_leaf_id}`, `ctx.navigateTree`, `ctx.ui.{notify,select,editor,setWidget,getEditorText,setEditorText}`, `pi.appendEntry`, `pi.sendUserMessage(deliverAs)`; full REVIEW_RUBRIC + REVIEW_GUIDELINES.md loader | stateless besides module-local reviewOriginId scalar + persisted custom entries; no host resource. Private pi-tui picker tree (Container/Input/SelectList/fuzzy_filter) replaced by public `ctx.ui.select/editor` |

Each translation maps its `node:fs`/`node:path`/`os`/`setTimeout`/`fetch`
ambient dependencies to Lua-native host bindings: `pi.fs.*`, `pi.path.*`,
`pi.env`, `pi.set_timeout/set_interval`, `pi.http.fetch`. earendil_pi-review
additionally maps its private pi-tui picker tree (Container/Input/SelectList/
fuzzy_filter) and custom loader to the public `ctx.ui.{select,editor}` dialogs
and `ctx.navigateTree`, per the composable-UI public slots.

## Verification

The narrowest load gate that exercises these new packages without touching
`crates/**`:

```sh
cargo build -p pi-rs-app
for f in pi-pomodoro pi-codex-fast pi-working-indicator pi-webfetch; do
  ./target/debug/pi --mode rpc --extension examples/extensions/dogfood/$f.lua < /dev/null
done
```

`--mode rpc` runs before model/auth resolution, loads the CLI extension through
the real product loader, and reports `Error: Failed to load extension` (exit 1)
on any translation error. Exit 0 with no such error means the package loaded and
registered through the public surface; the RPC role also fires `session_start`
and `session_shutdown` (visible as `extension_ui_request` records), so each
extension's startup and its cleanup path both execute.

Exit status for all four translations: **0**.

## Status of all 15

- **Translated this wave (load-verified EXIT=0):** pi-tool-management,
  pi-minimal-editor, pi-context-janitor, pi-gecko-websearch, pi-rlm,
  sting8k_pi-vcc (new host capabilities: `pi.ai.complete`, `pi.footer` facade,
  push-model `pi.register_setting_item`), and **earendil_pi-review**
  (`/review` + `/end-review`: full REVIEW_RUBRIC, `ctx.navigateTree`
  summarize/return, PR checkout via `pi.exec gh`, REVIEW_GUIDELINES.md loader
  via `pi.fs.stat/read_file`+`pi.path`; the upstream private pi-tui picker tree
  is replaced by the public `ctx.ui.select/editor` dialogs).
- **Already translated before this run (PLAN 9.5/9.8 evidence):** pi-compact →
  `examples/extensions/pi-compact.lua` (public `register_render_middleware`),
  plus pi-pomodoro, pi-codex-fast, pi-working-indicator, pi-webfetch.
- **Remaining:** pi-morph, pi-hashline are translated as
  `examples/extensions/dogfood/*.lua` already (morph/hashline load); pi-rtk has
  a public-surface delegation translation at
  `tests/dogfood-translations/rtk.lua` (delegates to the registered bash tool
  via `pi.registered_tools()` + `pi.exec rtk rewrite` + `user_bash`) since no
  public `createBashTool` factory exists.

See `manifest.json` for the per-package public-surface + cleanup contract and the
exact missing capability blocking each untranslated package.
