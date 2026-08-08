# Public Lua surface

pi-rs has exactly three public Lua surface tiers. The tiers classify compatibility and delivery; they do not create different trust or privilege levels. A capability is available only when its owning inventory row is `implemented`—listing a target here does not claim unfinished PLAN work is complete.

## 1. Pi-compatible API

The **Pi-compatible API** is the Lua translation of Pi v0.79.0's public extension contract: `ExtensionAPI`, event and context values, UI operations, registration behavior, discovery rules, and other extension-visible outcomes. Lua uses idiomatic spellings where documented (for example, `registerTool` → `register_tool`) while preserving Pi behavior.

`EXTENSION_INVENTORY.md` is the closed inventory for this tier. Its source-derived rows and Pi differential fixtures establish compatibility; they do not inventory additive pi-rs mechanisms or reusable Lua libraries.

## 2. Additive mechanism API

The **additive mechanism API** is pi-rs's Lua-native host capability superset. It exposes mechanisms needed to construct the shipped product and the maintained dogfood packages when Pi's extension API or ambient Node runtime does not provide a suitable Lua contract. Examples include process, filesystem, network, crypto, terminal, agent, session, cancellation, and lifecycle primitives.

These APIs promise their documented Lua contract, not Node module emulation or Pi product behavior. Each addition needs an owner in the construction or external-capability inventory plus a file-backed exerciser, translated Pi example, or dogfood consumer. Additive mechanisms may not change the default Pi-compatible product.

## 3. Packaged Lua modules

**Packaged Lua modules** are versioned, reusable Lua libraries distributed with builtin or user packages through the public module/dependency mechanism owned by PLAN 9.7. They hold composable Lua policy and helpers—such as tool factories, session/compaction helpers, and rendering utilities—rather than adding hidden host powers.

A module may use the Pi-compatible and additive APIs, but any host capability it needs must already be public in one of those API tiers. Embedded and file-backed packages resolve the same declared module graph. Chunk-local helpers, concatenation-order globals, and undeclared cross-pack globals are not packaged modules and do not count as public authoring surface.

The shipped exact-version public modules (defined with `pi.module.define`, imported with `pi.module.require`, identical for embedded and file-backed packages):

| Module | Version | Source unit | Exports |
|---|---|---|---|
| `pi.tools.prelude` | 1 | `builtins/tools/prelude.lua` | `split`, `fmt_num`, `utf8_lossy`, `cwd` |
| `pi.tools.truncate` | 1 | `builtins/tools/truncate.lua` | `truncate_head`, `truncate_tail`, `truncate_line`, `format_size`, limits |
| `pi.tools.path-utils` | 1 | `builtins/tools/path-utils.lua` | path normalization/resolution helpers |
| `pi.tools.mime` | 1 | `builtins/tools/mime.lua` | MIME detection helpers |
| `pi.tools.shell` | 1 | `builtins/tools/shell.lua` | `shell_config` |
| `pi.tools.output-accumulator` | 1 | `builtins/tools/output-accumulator.lua` | `new_output_accumulator` |
| `pi.tools.keybinding-hints` | 1 | `builtins/tools/keybinding-hints.lua` | key text/hint renderers, `HINT_KEYBINDINGS` |
| `pi.tui.visual-truncate` | 1 | `builtins/tools/visual-truncate.lua` | `truncate_to_visual_lines` |
| `pi.tools.render` | 1 | `builtins/tools/render-utils.lua` | ANSI/binary/path/line render helpers, `highlight_code` |
| `pi.tools.diff` | 1 | `builtins/tools/diff.lua` | diff line parsing and rendering |
| `pi.tools.edit-diff` | 1 | `builtins/tools/edit-diff.lua` | edit matching/application and diff generation |
| `pi.tools.file-mutation-queue` | 1 | `builtins/tools/file-mutation-queue.lua` | per-file mutation queue policy |
| `pi.utils.syntax-highlight` | 1 | `builtins/utils/syntax-highlight.lua` | `theme_highlight_code`, `markdown_highlight_code` |
| `pi.utils.messages` | 1 | `builtins/utils/messages.lua` | `bash_execution_to_text`, `convert_to_llm`, `convert_to_llm_with_block_images` |
| `pi.utils.extensions` | 1 | `builtins/utils/extensions.lua` | `context_policy`, `headless_ui` |
| `pi.utils.branch-summary` | 1 | `builtins/utils/branch-summary.lua` | branch summary/token estimation exports |
| `pi.utils.system-prompt` | 1 | `builtins/utils/system-prompt.lua` | `build_system_prompt`, `build_session_system_prompt`, context-file loading |
| `pi.utils.agent-session` | 1 | `builtins/utils/agent-session.lua` | `persist_agent_event`, `session_startup`, `construct_session` |
| `pi.utils.bash-executor` | 1 | `builtins/utils/bash-executor.lua` | interactive bash execution policy |
| `pi.utils.export-html` | 1 | `builtins/utils/export-html.lua` | `generate`, export rendering helpers |

`examples/extensions/module-demo.lua` is the file-backed consumer proof: it imports the modules above through the same `pi.module.require` path the builtin packs use and verifies the full registry (`pi.module.list`).


## No embedded/private tier

There is no embedded/private tier. `include_str!`, a synthetic `<pack:…>` source key, or builtin-package membership records provenance only. It must not change API-table members, module visibility, declaration semantics, precedence, snapshots/actions, watchdog treatment, or runtime/session/dispatch lifecycle.

Consequences:

- builtin policy may use only the same three tiers available to ordinary file-backed packages;
- source-name checks cannot unlock capabilities or bypass public declarations;
- internal Rust functions, host registries, and chunk-local Lua helpers are implementation details, not a fourth authoring tier;
- a builtin-only helper must become a packaged Lua module or remain an open construction-inventory defect;
- any current embedded/file-backed difference is unfinished PLAN work, not an API promise.

## Inventory terminology

Keep inventory claims distinct:

| Inventory | What it classifies | Surface tier evidence |
|---|---|---|
| Pi compatibility | Pinned Pi extension members, events, contexts, UI, loader rules, and examples | Tier 1 compatibility |
| First-party construction | Every shipped policy unit and Rust launch/composition seam | Public declaration/module use across tiers 1–3; no private bypass |
| External-extension capability | Pi API use plus package, process, network, filesystem, crypto, timer, lifetime, and concrete-class needs from the pinned dogfood suite | Tier 1 uses and tier 2/3 requirements |

Construction and capability rows may reference Pi-compatible API members, but must not duplicate the member-level compatibility inventory. Packaged modules are distribution and reuse contracts, not evidence that their underlying host mechanisms are implemented.
