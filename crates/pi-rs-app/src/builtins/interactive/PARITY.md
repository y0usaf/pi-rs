# Interactive shell source-parity manifest

Reference: `ref/pi` v0.79.0 at `c5582102`. This manifest records which Pi
sources back the current interactive shell and what each still lacks. Open
gaps are acceptance gaps for PLAN items 2–3, not optional polish. Product
behavior is embedded Lua; Rust entries are mechanisms only.

| Reference source | pi-rs destination | Status |
|---|---|---|
| `modes/interactive/theme/{dark,light}.json` | `builtins/interactive/theme/` | exact assets |
| `modes/interactive/theme/theme.ts` | `builtins/interactive.lua` | exact built-in dark/light truecolor/256-color loading + `/settings` preview/cancel/commit switching; custom/registered theme loading, detection, and watching join configuration/extensions (item 9) |
| `core/keybindings.ts` (idle/startup) | `builtins/interactive.lua` | exact defaults consumed by startup hints; config migration/reload open |
| `components/keybinding-hints.ts` | `builtins/interactive.lua` | exact formatting for startup hints |
| `components/footer.ts` | `builtins/interactive.lua` | exact formatting core and width behavior; live session/provider/git adapters open |
| `interactive-mode.ts` startup header | `builtins/interactive.lua` | exact ExpandableText compact/expanded assembly toggled by app.tools.expand; cell-pinned by `shell-turn` |
| `interactive-mode.ts` key handlers (`setupKeyHandlers`, `handleCtrlC/D`, `handleFollowUp`, `handleDequeue`, `handleClipboardImagePaste`, `setupExtensionShortcuts`) | `builtins/interactive.lua` | exact escape-abort/queue-restore, press-again exit, follow-up/dequeue, paste insert, shortcut routing; suspend, thinking cycle/toggle, external editor, session actions, bash-mode branches open (items 5/6/7); OS clipboard mechanism open (images milestone) |
| `interactive-mode.ts` containers (`init()` composition, `updatePendingMessagesDisplay`, working `Loader`, `restoreQueuedMessagesToEditor`) | `builtins/interactive.lua` (+ `pi-rs-tui/src/loader.rs` mechanism) | exact container order, queue rows + hint, loader lifecycle on agent_start/agent_end; retry/compaction loaders and widget/extension slots open (items 4/6/9) |
| `components/custom-editor.ts` | `builtins/interactive.lua` | exact input-routing policy: extension shortcuts, paste, interrupt/autocomplete, empty-only exit, app actions, editor fallback |
| `packages/tui/src/components/editor.ts` | `pi-rs-tui/src/editor.rs` | multiline editing, navigation, history, undo, paste, and autocomplete mechanisms; async completion debounce open |
| `packages/tui/src/tui.ts` | `pi-rs-tui/src/tui.rs`, `pi.tui.session`, `pi.tui.process_session` | async process scheduling, lifecycle, differential rendering, resize, cleanup, and live hardware-cursor/clear-on-shrink/progress controls; overlays open |
| `core/footer-data-provider.ts` | `builtins/interactive.lua` | model/context/usage adapters; git watcher, statuses, session/provider breadth open |
| `interactive-mode.ts` lifecycle | `builtins/interactive.lua`, `pi-rs-tui/src/process.rs` | start/input/resize/signals/stream concurrency/cleanup; suspend open |
| `main.ts` no-prompt selection | `pi-rs-app/src/main.rs` | TTY no-prompt selection; startup session-selection breadth open |
| `core/agent-session.ts` event bridge | `builtins/interactive.lua` | direct Agent bridge for streamed text, thinking, tools, persistence, abort; queued-text tracking mirrors `_steeringMessages`/`_followUpMessages` incl. message_start consumption; compaction queue open (item 6) |
| `modes/interactive/components/*` (user/assistant/thinking/tool) | `builtins/interactive.lua` | ported and cell-pinned (basic/tool/highlight/markdown turns); bash-execution, custom/compaction/branch-summary components open (items 5/6/7) |
| `components/settings-selector.ts`, `packages/tui/src/components/settings-list.ts` | `builtins/interactive.lua`, `pi-rs-tui/src/settings_list.rs` | `/settings` routing, search/windowing/descriptions, all value callbacks, warnings/thinking/theme submenus, live theme preview, and persistence cell-pinned by `settings-turn`; package-manager `ConfigSelectorComponent` and exported-but-product-unreachable legacy Theme/ShowImages selectors join item 9's compatibility inventory |
| `components/scoped-models-selector.ts`, `interactive-mode.ts` `showModelsSelector` | `builtins/interactive.lua` | `/scoped-models` presentation/input, live ordered session scope, Ctrl+P cycling, canonical-ID persistence + startup restore cell-pinned by `scoped-models-turn`; arbitrary configured glob/thinking patterns join item 9 |

Before closing a PLAN slice that touches this shell, update the affected rows;
an unported component, command, event, or edge state must not disappear
between headings.
