# pi-rs — execution plan

`DESIGN.md` defines the target: a Rust port of Pi v0.79.0's coding agent with
an exhaustive, closed difference list. `ref/pi` @ `c5582102` is the frozen
product specification outside those differences. This plan covers the coding
agent and only its required AI/auth, agent, session, TUI, and extension
mechanisms. Product-specific work belongs in downstream forks.

The first unchecked item is next. Completed implementation diaries remain in
Git; this file keeps only the current contract, concise completed ledger, and
actionable work.

## Execution axioms

These combine `DESIGN.md`, the applicable project doctrines, and the Rust/Lua
closure goal:

1. **The pinned Pi release is the oracle.** Outside DESIGN's closed exception
   list, observable behavior is exact—not approximate, improved, or redefined by
   a pi-rs-only test.
2. **The product boundary is closed.** Port only the coding agent and mechanisms
   it actually exercises. Unrelated framework breadth and product experiments
   stay downstream.
3. **Maintained executable source converges to Rust + Lua.** Rust owns mechanism;
   Lua owns product policy, configuration, builtins, and extensions. First-party
   tests, generators, and maintenance tools also converge to Rust or Lua.
   Nix/TOML manifests and inert data, docs, markup, styles, certificates, and
   protocol fixtures are not executable-language exceptions. Upstream Pi and
   external-extension source are pinned oracle inputs, not maintained copies.
   Browser-target JavaScript required by Pi-compatible standalone HTML export is
   the sole target-runtime exception: keep it minimal, provenance-marked, and
   incapable of becoming a host extension/tooling runtime. At closure there is
   no maintained host-side TypeScript, JavaScript, Python, or shell source.
4. **Extension-first has no privileged path.** Every replaceable first-party
   behavior is an independently ablatable Lua builtin declared through the same
   public surface as an ordinary file-backed extension. Synthetic source identity
   is provenance only.
5. **Snapshots enter; actions leave.** Lua reads immutable event/context
   snapshots and returns or queues actions. It never borrows mutable host state;
   every dispatch is watchdog-bounded and every owned resource has explicit
   cancellation/disposal.
6. **One kind, one declaration mechanism.** Applications, tools, commands,
   renderers, slots, resources, settings, and other repeated units have one
   public declaration path. Rust selects generic roles and applies actions; it
   does not name product policy.
7. **The bare core is real and continuously proven.** Zero packs/config/
   extensions still provide the documented minimal raw completion/login/model
   behavior. Per-package ablation and ordinary file-backed replacement prove the
   mechanism/policy line.
8. **Evidence must earn its carrying cost.** Keep the smallest deterministic
   test that distinguishes the contract. Unit tests, differential oracles,
   snapshots, exercisers, inventories, and audits coexist only when each catches
   a distinct failure. Generated evidence is canonical and compact; migration
   scaffolding expires when its gate closes; Git is the attic.
9. **Nix is the source of truth.** Build and verification claims use `nix build`
   and `nix flake check`. Native commands are iteration aids only; `cargo fmt`
   and `cargo clippy` remain sanctioned direct exceptions.

## Completed baseline

Implementation inventory, not a claim that open parity gates are complete:

- [x] Rust workspace, crane flake, bare host, Lua runtime/registries/watchdogs,
      terminal cell renderer/input/components, AI/auth transport, agent loop,
      session persistence, and thin generic role launcher.
- [x] **1–3: interactive foundation.** Pi-derived differential terminal harness;
      exact transcript, editor, autocomplete, shell, selectors, `/login`, `/logout`,
      and `/model`; jsdiff, tool renderers, syntax highlighting, marked edge cases,
      and ordered JSON behavior.
- [x] **4–5: coding-agent loop.** Anthropic/provider UI differential; system
      prompt/context, coding-tool semantics, images, and agent event ordering.
- [x] **6: sessions.** Persistence, resume/reconstruction, session UI, tree/
      branching, and compaction through the shipped product.
- [x] **7: interactive surface.** Bash mode, thinking, settings/scoped models,
      catalog updates, trust, subscription auth breadth, transfer/info commands,
      retry presentation, and remaining shell actions.
- [x] **9.1: product-loaded extensions.** Ordinary Lua extensions load through
      CLI/project/global/configured sources with trust, rollback, conflicts,
      watchdogs, active tools/commands/flags, autocomplete, and queued select/
      confirm/notify actions. Pi-generated runtime/context/UI observations and
      translated examples established the vertical slice.
- [x] **9.1a: closed construction and dogfood inventories.** Fail-closed manifests
      classify first-party assembly and the capabilities of 15 pinned external
      extensions; `LUA_SURFACE.md` defines compatible, additive-mechanism, and
      packaged-module tiers.
- [x] **9.1b: generic public assembly.** `pi.register_role`, one declarative
      builtins manifest, zero-pack boot, per-package suppression, and ordinary
      file-backed role/tool/policy replacements removed privileged product launch
      paths. This is the last completed plan step before the ablation milestone.

## Immediate milestone — evidence ablation and Rust/Lua closure

The parity push accumulated useful but excessively duplicated migration
scaffolding: 206 MiB of per-cell UI JSON, bespoke TypeScript oracle drivers,
checked copies of external TypeScript extensions, Python inventory generators,
and shell wrappers. Preserve the contracts, not their current representation.
Complete these rungs before growing the extension surface further.

- [ ] **A.1 Compact exact UI evidence without reducing coverage.** Replace the
      one-object-per-cell `tests/ui-parity/*.pi.json` format with one versioned,
      reviewable canonical format using a shared style palette, text/style runs,
      and frame deltas or an equally compact representation. Decode to the same
      complete cell grid and cursor state before comparison; retain color,
      attributes, wide cells, trailing blanks, geometry, ordering, input, and
      first-mismatch diagnostics. Delete old snapshots and conversion-only code.

      **Accept:** every retained Pi/pi-rs checkpoint compares identically before
      and after conversion; negative controls still identify the first mismatched
      cell; regeneration is byte-idempotent; tracked UI oracle bytes fall by at
      least 95%; and no compressed opaque blobs are committed.

- [ ] **A.2 Deduplicate the permanent test contract.** Classify retained evidence
      by the distinct failure it owns: Rust mechanism invariant, Pi differential,
      public Lua exerciser, construction/ablation proof, or external dogfood
      contract. Delete overlapping expectations, copied setup, stale milestone
      audits, and generated review output without an open owner. Factor terminal,
      HTTP/SSE, process, normalization, and fixture machinery once. Prefer
      black-box product boundaries over copied upstream private wiring; retain
      component/unit tests only when they localize a distinct failure.

      Active fail-closed inventories may remain while they drive open 9.x rows,
      then retire or collapse to minimal permanent manifests. Scenario count is
      not a success metric; unique observable contracts and mutation-resistant
      failure signals are.

      **Accept:** every retained suite states its unique contract and why a
      cheaper layer cannot own it; shared harnesses have one implementation;
      default checks contain no migration-only final audit; and the retained
      suite passes through the flake.

- [ ] **A.3 Close maintained executable source to Rust/Lua.** Port repository-
      owned TypeScript, Python, and shell generators/tools to Rust or Lua and
      delete their predecessors. Replace checked external-extension TypeScript
      trees with a hash-locked Nix oracle input plus compact checked provenance/
      capability manifests. Drive upstream Pi through shared Rust/Lua black-box
      harnesses where possible; any irreducible adapter belongs to the pinned
      external oracle input, not first-party product/test source. Separate opt-in
      oracle regeneration from normal offline verification: normal checks consume
      canonical outputs and execute no repository-owned Node/Bun/Python/shell
      program.

      Keep only explicitly allowlisted browser-export JavaScript and its
      provenance-marked third-party libraries. It executes only in generated
      standalone pages and cannot become an extension, package, generator, test
      harness, or host dependency. Add a Nix source-language check over tracked
      executable files and shebangs.

      **Accept:** the gate rejects new first-party `.ts`, `.py`, `.sh`, Python/
      shell shebangs, and `.js` outside the browser-export allowlist; model-catalog,
      inventory, audit, and oracle workflows have Rust/Lua owners; normal
      `nix flake check` needs no repository-owned foreign-language runtime;
      opt-in regeneration remains deterministic against hash-pinned sources; and
      shipped extension/config/package execution is Lua-only.

## Extension/configuration closure

- [ ] **9.2 Extension contexts + lifecycle actions.** Complete live
      `ExtensionContext`/`ExtensionCommandContext` snapshots and queued actions:
      UI, mode/hasUI/cwd/trust, read-only session/model registry, model/signal,
      idle/abort/pending/shutdown, context usage, compaction, system prompt, and
      command-only wait/new/fork/tree/switch/reload operations. Rebind contexts
      across reload/session replacement so stale handles fail without exposing
      mutable Rust state.

      Already landed: TUI/one-shot context snapshots, read-only facades,
      generation-based stale rejection, queued abort/compact/shutdown/wait, and
      command lifecycle actions including session replacement/reload.

      **Remaining/accept:** carry contexts through JSON/RPC delivery; cover
      signal-driven cancellation of queued/in-flight waits; and pin context,
      replacement, cancellation, stale-handle, and lifecycle/event ordering
      against Pi. Event emission itself closes in 9.3.

- [ ] **9.3 Complete event pipeline and fold semantics.** Emit the pinned event
      vocabulary at real product seams: project/resources; session start/switch/
      fork/compact/tree/shutdown; context/provider request/response; agent/turn/
      message/tool lifecycles; model/thinking selection; `tool_call`,
      `tool_result`, `user_bash`, and `input`. Port exact ordering, replacement vs
      mutation, middleware chaining, cancellation/fail-safe rules, error
      isolation, and result merges. No product-only callback path.

      **Accept:** one Pi differential covers successful/tool-using, blocked,
      transformed-input, bash, compact/tree/session-switch, provider-failure,
      abort, and reload paths; Lua sees equivalent snapshots and produces
      equivalent requests, final state, and transcript.

- [ ] **9.4 Complete non-UI ExtensionAPI actions and registries.** Finish dynamic
      tools/active-tool changes, async argument completion, shortcut conflicts,
      CLI flags, custom messages/render/persistence, session name/labels,
      command/tool inventories, model/thinking mutation, shared event bus, and
      provider register/unregister with custom stream/OAuth callbacks. Registered
      tools participate in prompt rebuilds, validation, parallel execution,
      renderer fallback, sessions, export, and reload exactly like builtins.

      **Accept:** translated dynamic-tools, tool-override, message-renderer,
      event-bus, preset, provider, and stateful-tool examples run unprivileged;
      focused differential contracts pin immediate effects and reload recovery.

- [ ] **9.5 Complete composable extension UI/rendering.** Expose Pi-equivalent
      select/confirm/input/editor dialogs, notifications, status/widgets,
      working message, header/footer, title, editor text/paste, tool expansion,
      theme access/switching, raw input, custom editor, and temporary custom
      component/overlay composition. Complete custom tool/message rendering,
      invalidation, focus, resize, cancellation, cleanup, and no-UI outcomes.

      Add ordered public rendering middleware for every transcript row kind plus
      declared header/footer/editor/status/widget slots. Middleware receives
      immutable snapshots and returns components/actions; errors fall through and
      dispatch remains watchdog-bounded.

      **Accept:** representative translated UI examples match Pi frames/input;
      one file-backed compact-rendering package reproduces `pi-compact` behavior
      without private classes; default middleware preserves retained UI parity.

- [ ] **9.6 Canonical `config.lua` declaration + mutation pipeline.** Provide one
      Lua declaration mechanism per kind: settings, keybindings, models/providers,
      themes, extensions, skills/prompts/resources, and selectors. Load global,
      then trusted project declarations with Pi-equivalent effective precedence
      and CLI overrides. Interactive mutation updates a deterministic managed Lua
      block idempotently; `/reload` publishes the whole next graph atomically.
      Pi JSON configuration inputs remain intentionally ignored.

      **Accept:** compact matrices cover precedence, trust, CLI overrides,
      failed/partial declarations, rollback, and repeated mutation round-trips;
      equivalent Lua declarations produce Pi-equivalent behavior and frames.

- [ ] **9.7 Resources, public Lua modules, and package transport.** Complete
      resource discovery/provenance/precedence/dedupe/toggles/reload for Lua
      extensions/config/themes and Pi-compatible skill/prompt content. Implement
      DESIGN's npm-registry, Git URL/ref, and local-path transports while package
      contents remain Lua/modules/data and JavaScript stays inert.

      Finish deterministic public modules for reusable policy—truncation,
      mutation queues, shell/tool/session/compaction/render/theme helpers—and
      remove undeclared chunk-local/cross-pack globals. Embedded and file-backed
      packages use the same dependency mechanism.

      **Accept:** resource/package fixtures cover precedence, trust, collisions,
      install/remove/list/update/config, toggles, offline cache, load order,
      cycles, and attribution; a file-backed package imports the same helpers as
      builtins without hidden native modules or a JS runtime.

- [ ] **9.8 Translation matrix + Pi extension gate.** Translate every in-boundary
      pinned first-party TypeScript extension example to executable Lua. Group
      truly equivalent examples, but never skip one because the bridge lacks a
      capability. Generate/check concise Lua API docs from the same minimal
      manifest.

      **Accept:** every pinned API member/event and configuration capability maps
      to differential evidence, executable Lua, or an explicit DESIGN exception;
      all in-scope examples run through the shipped public surface.

- [ ] **9.9 Inventory-driven Lua mechanism supersurface.** Implement only the
      low-level capabilities owned by construction/dogfood rows: abort-aware HTTP
      streaming, managed subprocess pipes/process-tree cancellation, TCP framed
      clients, filesystem watch/atomic/symlink/metadata operations, reviewed
      hashes/crypto, scoped tasks/timers/resources, reusable tool operations, and
      per-file mutation queues. Use Lua-native APIs, not Node emulation.

      Opaque handles may own external resources but never mutable product state.
      Product mutation remains queued; embedded/file-backed capabilities are
      identical; each operation has cancellation, timeout, reload, shutdown, and
      leak contracts.

      **Accept:** file-backed examples exercise every mechanism; no process/task/
      socket/watcher survives disposal; Gecko, RLM, Pomodoro, Hashline, Morph,
      and Webfetch need no missing-primitive shell workaround; default Pi
      behavior remains unchanged.

- [ ] **9.10 Close first-party decomposition, ablation, and replacement.** Resolve
      every construction row as an independently disableable public Lua builtin
      or an irreducible Rust mechanism recorded in DESIGN. Split replaceable
      frontend/agent/tool units; consume public event/render/slot/command/resource/
      lifecycle registries; remove product callbacks, local registries, hardcoded
      precedence, and private globals. Do not force singular mechanisms into
      ceremonial registries.

      **Accept:** zero-pack boot; per-package ablation; ordinary file-backed
      replacements for application role, agent policy, each tool kind, compaction,
      command routing, every render/slot kind, theme, and resources; deleting the
      builtins tree leaves the documented bare core; no open construction row.

- [ ] **9.11 Translate external dogfood and close the strict-superset gate.**
      Translate the 15 pinned packages—codex-fast, Gecko websearch, RTK, compact,
      context janitor, Morph, tool management, Webfetch, Hashline, minimal editor,
      working indicator, Pomodoro, RLM, review, and VCC—to ordinary Lua packages.
      Preserve behavior with the smallest deterministic provider, browser/socket,
      subprocess, timer, filesystem, compaction, session, and terminal contracts.
      Pi 0.80.6 is only the extension-behavior oracle, not the product spec.

      **Accept:** direct/configured/bundled loading composes identically; long-lived
      resources clean up; stateful packages survive branch/compact/reload/session
      replacement; `pi-compact` uses public middleware; no translation has a
      privileged escape hatch; compact inventories close; default Pi parity stays
      green.

## Remaining AI/auth and modes

- [ ] **8. Complete coding-agent AI/auth compatibility.** Keep one shared
      transport/conversion pipeline per protocol family rather than provider
      clones. Already landed: Anthropic, OpenAI Completions baseline, OpenAI
      Responses, Codex Responses SSE/WebSocket/fallback, Azure Responses, Google
      Generative AI, and Google Vertex including authorized-user, service-account,
      workload file/URL/executable/AWS ADC paths. Catalog dispatch currently
      covers those families and subscription auth breadth is complete.

      **Remaining:** certificate external-account ADC; deterministic Pi
      differentials and dispatch for `mistral-conversations` and
      `bedrock-converse-stream`; replace the old OpenAI Completions fixtures with
      one Pi differential; run catalog/auth acceptance and delete superseded
      provider fixtures/harnesses under A.2.

      **Accept:** supported model inventory matches Pi's coding agent; every
      advertised API has a focused deterministic replay; three subscription
      providers retain auth-state/request coverage; shared machinery has one
      implementation.

- [ ] **10. Match non-interactive modes.** Port print, JSON, RPC, export, and other
      pinned coding-agent modes through generic registered roles and the same Lua
      policy/actions as interactive mode.

      **Accept:** argument, stdout/stderr, exit status, serialization, extension
      context/action delivery, cancellation, and no-UI outcomes match Pi.

- [ ] **11. Final parity and ablation audit.** Diff the complete reachable
      coding-agent surface and required AI/agent/TUI behavior. Resolve every
      difference outside DESIGN; verify each listed exception is no broader than
      stated. This is a product contract check, not a new permanent audit layer.

      **Accept:** retained automated contracts and side-by-side scripted sessions
      are indistinguishable under equivalent Lua configuration; inventories have
      closed and collapsed to minimal permanent manifests; zero/per-pack ablation
      and ordinary replacement are green; maintained executable source satisfies
      the Rust/Lua gate; no migration-only audit remains. Tag the baseline.

## Post-parity maintenance

Maintain the frozen compatibility contract and deliberately port selected
upstream changes. Checked contracts stay small and Rust/Lua-driven; foreign
upstream source is realized only as a hash-pinned input for deliberate oracle
regeneration. Delete adapters, snapshots, manifests, and audits whenever a
smaller permanent contract supersedes them. Product-specific defaults and
experiments remain downstream.

## Execution mechanics

- The first unchecked item is next; close its checkbox and acceptance evidence in
  the same change.
- No temporary UI, approximate component, knowingly different default, or
  pi-rs-specific label satisfies a milestone.
- A public authoring capability needs one outside-the-builtins consumer: a
  file-backed example, translated pinned example, or maintained dogfood package.
  Add another layer only for a distinct failure mode.
- Use focused native tests while iterating. Completion claims cite relevant Nix
  checks; releases run the complete flake verification.
