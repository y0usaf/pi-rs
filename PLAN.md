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

- [x] **A.1 Compact exact UI evidence without reducing coverage.** Replace the
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

      **Status — source-language gate (landed).** The gate ships as a Rust binary
      (`crates/pi-rs-tools`, `pi-rs-tools gate {scan,update-manifests,check}`) and
      is wired into `nix flake check` as `checks.source-language-gate`. It rejects
      new first-party `.ts`/`.py`/`.sh`/shebang files and `.js` outside the
      browser-export allowlist, while the current footprint is frozen in
      `tests/source-language/legacy.json` (grandfathered) and
      `tests/source-language/allowlist.json` (3 browser-export JS). Normal checks
      run the gate with no Node/Bun/Python runtime.

      **Status — construction inventory (landed).** `scripts/construction-inventory`
      (Python) and `tests/construction-inventory/test_checker.py` are ported to
      Rust: `pi-rs-tools construction-inventory {--check,--print-extracted}` plus
      an offline negative-control `selftest` that mirrors the Python unittest.
      The flake `construction-inventory` check now runs the Rust binary with no
      repo-owned Python; both Python predecessors are deleted and withdrawn from
      `tests/source-language/legacy.json`.

      **Remaining — port the named workflows.** The grandfathered generators are
      still Python/bash/TS and must gain Rust/Lua owners before A.3 closes.
      Model-catalog has closed: `scripts/update-model-catalog.ts` → Rust
      (`crates/pi-rs-tools`, `pi-rs-tools model-catalog {update,selftest}`), its
      bash fixture harness deleted, and the flake check/app repointed to the Rust
      binary with no bun. Remaining:
      - `scripts/{final-parity-audit,extension-inventory,
        external-extension-inventory,dogfood-oracle}` (Python) → Rust/Lua.
      - the `scripts/*-oracle` bash regen wrappers + `gen-arch.sh` → Rust.
      - `tests/*/gen-oracle.ts`, `tests/ui-parity/pi-*.ts` → Rust/Lua harnesses.
      - `tests/external-extension-inventory/fixtures/**` (external TS) →
        hash-locked Nix oracle input + provenance/capability manifests.
      Each port deletes its predecessor and, when the set is complete, removes it
      from `tests/source-language/legacy.json` so the gate's grandfathered set
      shrinks to zero. Oracle regeneration stays opt-in against hash-pinned
      sources; normal offline checks consume canonical outputs.

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

      **Remaining/accept:** carry contexts through JSON/RPC delivery; and pin
      context, replacement, cancellation, stale-handle, and lifecycle/event
      ordering against Pi. Signal-driven wait cancellation is closed: a queued
      `waitForIdle` resolves (not throws, not hangs) once the agent becomes
      idle even on abort, pinned against a Pi-generated oracle section
      (extension-context-parity `waitCancellation`) asserted in
      `crates/pi-rs-app/tests/extension_loading.rs`
      (`extension_context_snapshots_and_shutdown_match_pi`). The RPC extension
      UI binding is closed: Pi's RPC binds a real `ExtensionUIContext`
      (rpc-mode.ts `createExtensionUIContext`), and pi-rs's RPC role now
      reports `extension_has_ui=true` and transports UI requests as
      `extension_ui_request` JSONL records on stdout, asserted differentially
      in `rpc_binds_real_extension_ui_context_matching_pi`. Event emission
      itself closes in 9.3.

- [x] **9.3 Complete event pipeline and fold semantics.** Emit the pinned event
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

      Closed by two Pi-generated differentials: `complete_event_folds_match_pi_runner_oracle`
      and `real_product_seams_follow_pi_generated_event_order` assert strict whole-output
      equality against `tests/extension-event-parity/oracle.json` for the complete event
      vocabulary, ordering, fold/middleware results, transformed input, blocked `tool_call`,
      compact/tree/session-switch, context, trust, resources, and error isolation (the latter
      also pins productTrace + extensionErrors at real startup/tool-using/provider seams through
      the print role on an SSE stub). `provider_failure_reload_and_abort_paths_follow_pi_seam_oracle`
      pins the three acceptance paths the fold oracle does not cover — provider-failure (auto-retry
      re-drive), abort (mid-stream signal), and reload (session replacement) — against
      `tests/extension-event-parity/seams-oracle.json`, reproduced through the real interactive
      AgentSession over the scripted streamFn seam with the shared 03-seams trace extension.
      Each Lua record's replacement-vs-mutation, error isolation, result-merge, and ordering
      semantics live in `crates/pi-rs-app/src/builtins/utils/extensions.lua` (`EXTENSION_POLICY`
      fold) and are dispatched at every product seam (coding-agent.lua print/rpc, interactive.lua).
      Regenerate both oracles with `scripts/extension-event-oracle`.

- [x] **9.4 Complete non-UI ExtensionAPI actions and registries.** Finish dynamic
      tools/active-tool changes, async argument completion, shortcut conflicts,
      CLI flags, custom messages/render/persistence, session name/labels,
      command/tool inventories, model/thinking mutation, shared event bus, and
      provider register/unregister with custom stream/OAuth callbacks. Registered
      tools participate in prompt rebuilds, validation, parallel execution,
      renderer fallback, sessions, export, and reload exactly like builtins.

      Already landed: provider `register/unregister` with a custom `streamSimple`
      handler — a registered provider carrying `api` + `streamSimple` publishes a
      custom API stream handler that `pi.ai.stream_simple` dispatches ahead of Rust
      providers (stream.ts resolveApiProvider equivalent), and unregister removes
      it; `crates/pi-rs-host/tests/providers.rs` pins dispatch and post-unregister
      fallthrough. `register_provider` now reproduces Pi's `validateProviderConfig`
      byte-for-byte (streamSimple-without-api, models baseUrl/credential/api
      checks) and stores no config on a failed registration, pinned by a
      Pi-generated differential oracle
      `tests/provider-registry-parity/oracle.json` replayed through the public Lua
      surface (`crates/pi-rs-host/tests/provider_registry_parity.rs`).
      Regenerate with `scripts/provider-registry-oracle`.

      Closed (this stream): the ExtensionAPI runtime action/view methods
      (`sendMessage`, `sendUserMessage`, `appendEntry`, `setSessionName`,
      `getSessionName`, `setLabel`, `getActiveTools`, `getAllTools`,
      `setActiveTools`, `refreshTools`, `setModel`, `getThinkingLevel`,
      `setThinkingLevel`) are now bound onto the shared `pi` table for each live
      session by `EXTENSION_POLICY.bind_pi_actions` (`utils/extensions.lua`,
      spec `runner.ts bindCoreActions` → `agent-session.ts`). Reads return
      immutable snapshots; mutations enqueue through the same queued-action
      pipeline as the `ctx.*` methods. `getAllTools` is backed by the new
      `pi.registered_tools_with_source` host mechanism (every tool plus
      `sourceInfo`). Both product modes rebind on startup and on `/reload`
      (`coding-agent.lua` print role; `interactive.lua` `bind_session_runtime`),
      so a stale handle from a replaced session rejects via the generation
      bump instead of mutating a dead session. `setActiveTools`/`refreshTools`
      rebuild the base system prompt to reflect the active tool set (Pi
      `setActiveToolsByName`/`_refreshToolRegistry`), so dynamically-registered
      tools participate in prompt rebuilds exactly like built-ins.

      Exercised unprivileged by `examples/extensions/runtime-actions-demo.lua`
      (the translated session-name/preset/dynamic-tools/message-renderer/
      send-user-message surface: tool inventory reads, active-tool writes with
      prompt rebuild, session name/label, custom-message persistence via
      `appendEntry`, model/thinking mutation, and a runtime-registered tool
      surfaced through `refreshTools`) and pinned by
      `bound_pi_runtime_actions_apply_immediately` in
      `crates/pi-rs-app/tests/extension_loading.rs`, which asserts immediate
      effects (reads, applied `setActiveTools`/`setSessionName`/
      `setThinkingLevel`/`setModel`/`appendEntry`/`sendMessage`) and reload
      recovery (fresh generation after `/reload`, bound methods still read the
      replaced session).

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

- [x] **9.6 Canonical `config.lua` declaration + mutation pipeline.** Provide one
      Lua declaration mechanism per kind: settings, keybindings, models/providers,
      themes, extensions, skills/prompts/resources, and selectors. Load global,
      then trusted project declarations with Pi-equivalent effective precedence
      and CLI overrides. Interactive mutation updates a deterministic managed Lua
      block idempotently; `/reload` publishes the whole next graph atomically.
      Pi JSON configuration inputs remain intentionally ignored.

      **Accept:** compact matrices cover precedence, trust, CLI overrides,
      failed/partial declarations, rollback, and repeated mutation round-trips;
      equivalent Lua declarations produce Pi-equivalent behavior and frames.

      Closed (differential): `tests/config-settings-parity/oracle.json` is
      generated from Pi's real `SettingsManager` (deep merge + `migrateSettings`
      for queueMode/websockets/skills-object/retry.maxDelayMs + the full typed
      getter read-model) and `KeybindingsManager` (`migrateKeybindingsConfig`
      legacy-name remap + order + `getResolvedBindings`). `crates/pi-rs-host/
      tests/config_settings_parity.rs` replays each scenario through pi-rs's
      canonical `config.lua` declaration surface and asserts Pi's typed getter
      outcomes and migrated keybinding maps byte for byte. The declaration +
      mutation pipeline (global-then-trusted-project precedence, CLI overrides,
      atomic rollback, idempotent managed-block persistence, deliberate JSON
      rejection, file-backed extension use of the same surface) is pinned by
      `crates/pi-rs-host/tests/config_pipeline.rs` and `settings_bindings.rs`.
      Regenerate with `scripts/config-settings-oracle`.

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

- [x] **9.9 Inventory-driven Lua mechanism supersurface.** Implement only the
      low-level capabilities owned by construction/dogfood rows: abort-aware HTTP
      streaming, managed subprocess pipes/process-tree cancellation, TCP framed
      clients, filesystem watch/atomic/symlink/metadata operations, reviewed
      hashes/crypto, scoped tasks/timers/resources, reusable tool operations, and
      per-file mutation queues. Use Lua-native APIs, not Node emulation.

      Opaque handles may own external resources but never mutable product state.
      Product mutation remains queued; embedded/file-backed capabilities are
      identical; each operation has cancellation, timeout, reload, shutdown, and
      leak contracts.

      Landed (`examples/extensions/*.lua` exercisers + host/app tests drive each):
      `pi.http.fetch` and abort-aware `pi.http.stream(on_chunk)`; `pi.process.spawn`
      with stdio pipes, process-tree SIGTERM/SIGKILL, and Drop-tree kill;
      `pi.tcp.connect` with read/write/close and Drop-socket disposal;
      `pi.fs` symlink/lstat/chmod/rename/access/copy/mkdtemp/remove_dir/remove_dir_all
      + atomic write + richer stat + pollable `watch_file`; `pi.crypto` (sha1/sha256/md5/xxhash/random_uuid)
      and `pi.buffer`; `pi.set_timeout/set_interval/clear_*`; the public
      `pi.tools.file-mutation` module. `LUA_SURFACE.md` documents the low-level
      register and the construction inventory closes the `module.file-mutation-queue`
      row. The dependent Gecko/RLM/Pomodoro/Hashline/Morph/Webfetch primitives are all
      available Lua-natively with no missing-primitive shell workaround; full dogfood
      translations and leak fixtures remain with 9.11.

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

- [x] **8. Complete coding-agent AI/auth compatibility.** Keep one shared
      transport/conversion pipeline per protocol family rather than provider
      clones. Anthropic, OpenAI Completions, OpenAI Responses, Codex Responses
      SSE/WebSocket/fallback, Azure Responses, Google Generative AI, and Google
      Vertex (authorized-user, service-account, workload file/URL/executable/
      certificate/AWS ADC paths), plus Mistral Conversations and Bedrock Converse
      Stream all route through one registered `api` dispatch
      (`crates/pi-rs-ai/src/registry/stream.rs`) with a single shared HTTP/SSE
      transport. Catalog dispatch covers those nine API families and three
      subscription OAuth providers; cert external-account ADC is closed.

      **Closed (deterministic Pi differentials + registry acceptance):** every
      advertised coding-agent API has a focused Pi-derived replay
      (`tests/{anthropic,openai-completions,openai-responses,openai-codex-websocket,
      azure-openai-responses,google-generative-ai,google-vertex,mistral-conversations,
      bedrock-converse-stream}-parity/oracle.json`). The certificate external-account
      ADC path (mtls subject-token + STS exchange) is a `google-vertex-parity`
      case (`adc-workload-certificate`). `mistral-conversations` and
      `bedrock-converse-stream` have Pi differentials (`tests/mistral-conversations-parity/`,
      `tests/bedrock-converse-stream-parity/`) plus registered dispatch
      (`stream_mistral`/`stream_simple_mistral`, `stream_bedrock`/
      `stream_simple_bedrock`), both asserted by `crates/pi-rs-ai/tests/
      mistral_conversations_parity.rs`, `.../bedrock_converse_stream_parity.rs`,
      and the registry (`crates/pi-rs-ai/tests/registry.rs`). The old hand-built
      OpenAI Completions fixtures were replaced by the ONE Pi differential:
      `tests/openai-completions-parity/` now also pins `tools: []` on bare
      tool-history and the `content_filter` finish-reason error, and the superseded
      `crates/pi-rs-ai/tests/openai_completions.rs` + `tests/fixtures/
      openai-completions/` deleted. Whole-catalog/subscription-auth acceptance:
      `crates/pi-rs-app/tests/ai_auth_catalog.rs`
      (`every_catalog_api_dispatches_and_subscription_auth_registry_is_complete`).

      **Accept:** supported model inventory matches Pi's coding agent; every
      advertised API has a focused deterministic replay; three subscription
      providers retain auth-state/request coverage; shared machinery has one
      implementation.

- [ ] **10. Match non-interactive modes.** Port print, JSON, RPC, export, and other
      pinned coding-agent modes through generic registered roles and the same Lua
      policy/actions as interactive mode.

      **Remaining/accept:** argument, stdout/stderr, exit status, serialization,
      extension context/action delivery, cancellation, and no-UI outcomes match Pi.

      Already landed (print text mode): `modes/print-mode.ts` text semantics —
      the final assistant message's text blocks are written to stdout each
      followed by `\n` (no delta streaming), and an `error`/`aborted` stop reason
      yields exit 1 with the message on stderr. `crates/pi-rs-app/src/builtins/
      coding-agent.lua` no longer streams `text_delta`s in text mode and returns
      `exitCode`/`stopReason`/`errorMessage`; `crates/pi-rs-app/src/main.rs` maps
      them to the process exit status and stderr, emitting exactly
      `errorMessage || \`Request ${stopReason}\`` like Pi's `console.error`.

      Already landed (`@file`/stdin/initial-message): `args.ts` `@file` parsing
      (`Args.fileArgs`), `file-processor.ts` `processFileArguments` (text + image
      auto-resize + dimension note + empty-file skip + missing-file error/exit),
      `main.ts readPipedStdin` + `prepareInitialMessage`, and `initial-message.ts`
      `buildInitialMessage` composition are ported in
      `crates/pi-rs-app/src/cli/file_processor.rs` and wired through
      `crates/pi-rs-app/src/main.rs`; RPC mode rejects `@file` like Pi. The text
      path is pinned byte-for-byte against a Pi-generated differential.
      `@file` image attachments flow into the print role's initial user message
      (`agent:prompt` images).

      Closed (differential): `tests/print-mode-parity/oracle.json` is generated
      from Pi's real `runPrintMode` and records Pi's raw stdout/stderr/exit for
      scripted assistant final messages. `crates/pi-rs-app/tests/print_mode_
      parity.rs` drives the same final messages through pi-rs's print role via a
      registered custom `streamSimple` provider (public Lua surface), captures
      `pi.output` bytes, and asserts byte-for-byte stdout plus the exit/stopReason/
      errorMessage→stderr mapping for single-prompt text cases, the JSON-mode
      header+per-event framing contract, and the `messages[]` follow-up sequence
      (each remaining CLI message sent as a sequential `session.prompt`, with
      only the final assistant message's text written to stdout and the exit
      code/error taken from the final message). Regenerate with
      `scripts/print-mode-oracle`.

      Closed (args/help differential): `tests/args-parity/oracle.json` is
      generated from Pi's real `parseArgs` + `printHelp` (cli/args.ts);
      `crates/pi-rs-app/tests/args_parity.rs` replays the same argv corpus and
      help text and asserts parse equality plus byte-for-byte help. The landing
      parser now mirrors Pi's semantics (including `-p`/`--print` consuming a
      following non-flag message, silent invalid-`--mode` handling, unknown
      `--long`-flag collection, single-dash unknown-option error diagnostics,
      and the full landed flag set). Regenerate with `scripts/args-oracle`.

      Closed (RPC framing + synchronous protocol): `--mode rpc` now dispatches
      (before model/auth resolution) to a faithful RPC role that emits Pi's
      exact JSONL framing — `{type:"response",command,success,data|error}` with
      `id` present only when the client sent one, unknown-command errors shaped
      like Pi, and the `parse` command on non-JSON input. The synchronous
      command vocabulary (get_state, get_available_models, set_steering_mode,
      set_follow_up_mode, set_thinking_level, cycle_thinking_level, set_model,
      cycle_model, set_auto_compaction, set_auto_retry, abort_retry,
      get_messages, get_last_assistant_text, get_session_stats, export_html,
      get_commands, set_session_name) is pinned semantically against Pi's real
      `runRpcMode` oracle (`scripts/rpc-oracle`, `tests/rpc-parity/oracle.json`)
      by `crates/pi-rs-app/tests/rpc_mode_parity.rs` driving the real `pi`
      binary as a subprocess.

      Closed (RPC empty-array serialization): Pi's `getUserMessagesForForking()`,
      `get_commands`, and `session.messages` (`get_messages`) return real
      arrays, so empty results serialize as `{messages: []}` / `{commands: []}`
      — not the `{}` pi-rs's empty Lua table produced. All three are now seeded
      from a decoded empty array and pinned byte-for-byte against Pi-generated
      oracle cases (`empty-fork-messages`, `empty-commands`, `empty-messages`)
      in `tests/rpc-parity/oracle.json` via
      `rpc_empty_fork_messages_matches_pi_byte_for_byte`,
      `rpc_empty_commands_matches_pi_byte_for_byte`, and
      `rpc_empty_messages_matches_pi_byte_for_byte`.

      Closed (RPC per-command async scheduling): Pi's `handleCommand` runs
      each RPC command as its own Node async task: synchronous commands (no
      `await` before their response) emit during input processing in arrival
      order, while await-involving commands defer emission to microtask
      completion — Pi's continuation ordering resolves them in ascending
      await-depth (a depth-1 command completes before a depth-2 one), FIFO
      among equal depth. The RPC role now reproduces this deterministically:
      it reads the full command stream, emits depth-0/sync responses inline in
      arrival order, then emits deferred (awaited) responses in
      ascending-await-depth/FIFO order. This is pinned against Pi's real
      oracle by new cases — `async-steer-followup-abort` (abort_bash is sync
      and emits first, then deferred steer/follow_up/abort FIFO) — in
      `tests/rpc-parity/oracle.json` via
      `crates/pi-rs-app/tests/rpc_mode_parity.rs`
      (`rpc_async_deterministic_commands_match_pi_oracle`).

      Boundary (recorded): the remaining RPC async agent-streaming commands
      that require concurrent agent/event streaming or scripted session data
      (prompt, bash, compact, fork, clone, new_session, switch_session,
      get_fork_messages) and the Node `RpcClient` stay open under PLAN 10 (the
      stdout output-guard is closed: stray extension `print`/`io.write` route
      to stderr so non-interactive stdout stays protocol-clean, RPC now
      loads CLI `--extension` files like Pi, and Pi's RPC extension-UI binding
      is closed — `ctx.hasUI==true`, real `ExtensionUIContext` transported as
      `extension_ui_request` JSONL records, per `createExtensionUIContext`;
      see `crates/pi-rs-app/tests/rpc_mode_parity.rs`
      `rpc_binds_real_extension_ui_context_matching_pi`,
      `rpc_stdout_guard_routes_extension_stdout_to_stderr` and
      `rpc_loads_cli_extension_files`); startup-ui also remains open. The
      toolCall-only `text-no-text-content` oracle
      case scripts Pi's *observed state* directly and pi-rs's real agent would
      continue its tool loop on stopReason `toolUse`, so it is not a faithful
      terminal print outcome and remains a PLAN 10 open row.

- [ ] **11. Final parity and ablation audit.** Diff the complete reachable
      coding-agent surface and required AI/agent/TUI behavior. Resolve every
      difference outside DESIGN; verify each listed exception is no broader than
      stated. This is a product contract check, not a new permanent audit layer.

      **Accept:** retained automated contracts and side-by-side scripted sessions
      are indistinguishable under equivalent Lua configuration; inventories have
      closed and collapsed to minimal permanent manifests; zero/per-pack ablation
      and ordinary replacement are green; maintained executable source satisfies
      the Rust/Lua gate; no migration-only audit remains. Tag the baseline.

      **Platform sub-rows (stream s09, closed by Pi-generated differentials):**
      `tui.platform-modifiers` — native modifier polling (`native-modifiers.ts`)
      + Apple-Terminal Shift+Enter normalization (`terminal.ts`) ported as
      `pi-rs-tui` mechanism exposed via `pi.tui`; `coding.platform-clipboard` —
      the optional native clipboard addon resolution core (`loadClipboardNative`)
      + module-level gate (`!TERMUX_VERSION && hasDisplay`) ported as
      `pi.clipboard`, then extended past the addon-unavailable fallback with the
      resolved-addon preference ordering honored by `read_image`/`write_text`;
      `coding.footer-git` — live git-branch discovery (`findGitPaths` +
      `resolveGitBranch`) ported as `pi.git.current_branch`, wired into the
      default footer (`interactive.lua` `footer_live_branch`), with the
      extension-status data provider flowing through `ctx.ui.setStatus`. Each is
      pinned against a Pi-generated oracle
      (`tests/platform-modifiers-parity/`, `tests/platform-clipboard-parity/`,
      `tests/footer-git-parity/`). `coding.platform-update` is recorded as a
      DESIGN platform boundary in `DESIGN.md` (native Rust binary ships no
      self-update, bundled native addons, or Bun empty-env failure). The final
      live side-by-side gate and `coding.assembly` remain with s14.

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
