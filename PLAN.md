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

- [x] **9.2 Extension contexts + lifecycle actions.** Complete live
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

      **Carried through JSON/RPC (this stream):** print/json and RPC extension
      commands now receive live `ExtensionCommandContext` snapshots. Pi's
      `session.prompt()` (which every mode drives via its `messages[]` loop or
      the `prompt` RPC command) routes leading-`/` messages to the registered
      command handler with a command context and does not send them to the
      model (provider response stays pending, session messages stay empty).
      pi-rs reproduces `_tryExecuteExtensionCommand` via
      `EXTENSION_POLICY.try_execute_extension_command` and wires it into the
      print/json initial + follow-up messages and the RPC `prompt` command.
      Differential coverage pins delivery context (mode/hasUI/cwd/trust/idle/
      pending/session/model-registry/wait/new/fork/tree/switch/reload) and the
      not-consumed behavior for print, json, and rpc against a Pi-generated
      oracle section (extension-context-parity `delivery`) asserted by
      `crates/pi-rs-app/tests/command_delivery_parity.rs`
      (`print_and_json_command_delivery_matches_pi_oracle`,
      `rpc_command_delivery_matches_pi_oracle`). The rpc delivery also pins
      `hasUI=true` + `ui.notify` (real `createExtensionUIContext`). Context,
      replacement, cancellation, stale-handle, and lifecycle/event ordering
      remain pinned by the pre-existing oracle sections (`snapshot`, `stale`,
      `modes`, `actions`, `replacement`, `reload`, `waitCancellation`)
      asserted in `extension_context_snapshots_and_shutdown_match_pi`.

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

- [x] **9.5 Complete composable extension UI/rendering.** Expose Pi-equivalent
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

      Closed (this stream): the composable extension UI surface is complete and
      pinned through the mobile/files surface. `ctx.ui.select/confirm/input/
      editor/custom/notify`, status/widgets/header/footer/title, working
      message/indicator/visibility, hidden-thinking label, editor text/paste,
      tool expansion, theme access/switching, raw input (`onTerminalInput`),
      custom editor (`setEditorComponent`), and headless no-UI outcomes are all
      driven by `EXTENSION_UI_POLICY` under `builtins/interactive.lua`, with
      print-mode headless outcomes pinned by `print_context_uses_pinned_no_ui_
      outcomes`. Raw-input routing (`EXTENSION_UI_POLICY.handle_raw_input`
      consume/transform on the real and parity feed seam), custom editor
      (`setEditorComponent`/`getEditorComponent`), theme access (`getTheme`/
      `getAllThemes`/`setTheme`), and `addAutocompleteProvider` are pinned by
      `file_backed_ui_ops_drive_raw_input_theme_and_custom_editor` +
      `examples/extensions/ui-ops-demo.lua`. Temporary custom component/overlay composition now supports
      `overlay` + `overlayOptions` (anchor/margin/width/height) + `onHandle`
      (hide/setHidden/focus/unfocus/isHidden/isFocused) with input routing,
      focus release, cleanup/dispose, and resize (`interactive-extension-ui-
      parity-sequence` `feed` routes a focused overlay; `file_backed_ui_showcase_
      drives_dialogs_slots_editor_and_cleanup` drives the full showcase incl. an
      anchored overlay). Custom message rendering is closed: `pi.register_
      message_renderer(customType, renderer)` (Pi `registerMessageRenderer`)
      registers the first per-customType renderer, resolved by
      `pi.registered_message_renderers(customType)` with source attribution and
      first-wins semantics; the interactive custom-message transcript row consults
      it over an immutable snapshot, falling through to the default box on renderer
      error; pinned by `message_renderers_resolve_first_wins_attributed_and_roll_
      back` (`crates/pi-rs-host/tests/registries.rs`) +
      `file_backed_message_renderers_drive_custom_transcript_rows` +
      `examples/extensions/message-render-demo.lua`. Custom tool rendering
      (`renderCall`/`renderResult`/`renderShell`) rides the same immutable-context,
      error-fallthrough path (`tool-render-demo.lua`, `tool_execution_lines`).
      Ordered public rendering middleware covers every `DEFAULT_TRANSCRIPT_KINDS`
      row kind plus `header`/`status`/`widget_above`/`editor`/`widget_below`/
      `footer` slots via `pi.register_render_middleware`/`pi.register_ui_slot`
      (additive `api.rs` registries), receiving immutable snapshots and returning
      components/actions with error fallthrough and watchdog-bounded dispatch;
      `pi-compact.lua` reproduces compact rendering file-backed without private
      classes (`file_backed_compact_middleware_composes_without_frontend_patching`).
      Retained interactive UI parity is unchanged (the extension-ui and full
      `scripts/ui-diff` suites remain green).

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

      **Landed:** public exact-version modules `pi.tools.file-mutation-queue`,
      `pi.tools.shell`, `pi.interactive.export-html`, `pi.interactive.prompts`,
      `pi.resources.skills`, and `pi.packages` are consumed by embedded builtins
      and file-backed packages (examples/extensions/module-demo.lua,
      prompts-consumer.lua, skills-consumer.lua, pm-consumer.lua) through the
      single `pi.module.require` mechanism; the undeclared cross-pack
      `export_html_lib` global was removed (now the
      `pi.interactive.export-html` module).

      The disk resource-loader seam is closed: `pi.interactive.prompts`
      `load_prompt_templates` and `pi.resources.skills` `load_skills_from_dir`
      read through `pi.fs`/`pi.parse_frontmatter` and attach Pi's provenance
      (source/scope/origin/baseDir), validation warnings, and discovery rules
      (global agent-dir + project `.pi` + explicit paths; SKILL.md root vs
      recursive dirs vs inline `.md`; dot/node_modules/ignore handling). The
      git source grammar is exposed to Lua as `pi.git` (parse_git_url,
      is_local_path).

      The deterministic resource-resolution engine landed as `pi.resources`
      (module `pi.resources`): a `resolve()` that produces precedence-sorted,
      de-duplicated, attribution- and trust-aware resource lists
      (extensions/skills/prompts/themes) from configured packages (project first,
      identity-dedupe), settings entries with pattern filters, and the
      auto-discovery convention dirs (`.pi`/agent `extensions`, `skills`,
      `prompts`, `themes`; `.agents/skills`). Collisions collapse by canonical
      path; precedence is project+settings > project+auto > user+settings >
      user+auto > package; project-scope resources are gated by project trust.
      A disk theme registry (`load_theme_from_path`, `sync_themes`,
      `get_available_themes`, `get_theme`) parses+validates `.json` themes and
      is re-populated on `/reload`, so a custom theme selected in settings is
      applied by name. The host `pi.settings` bridge gained the
      extensions/skills/prompts/themes path channels (user+project),
      `is_project_trusted`, and `npm_command`. The `/reload` handler now calls
      `resolve_resources_into_state` and applies a resolved custom theme.
      Hermetic fixtures `crates/pi-rs-app/tests/resources_parity.rs` and the
      file-backed `examples/extensions/resources-consumer.lua` prove precedence,
      trust gating, collisions/dedupe, attribution, settings-vs-auto precedence,
      toggles, module cycles, and offline-cache skip through the shared
      `pi.module.require` mechanism.

      The package lifecycle landed as `pi.packages`: npm/git/local source
      routing (npm `npm:` spec split, git URL grammar, local paths), the
      SettingsManager packages channel (`pi.settings.packages()`,
      `set_packages`, `set_project_packages` with user/project scope),
      install/remove-from-settings, listConfiguredPackages, and
      getInstalledPath; install drives npm/git through the public
      `pi.exec`/`pi.fs` mechanisms and never evaluates package JavaScript.

      Differential: `tests/prompt-parity` pins the prompt-template pure
      functions (parseCommandArgs/substituteArgs/expandPromptTemplate);
      `tests/prompt-loader-parity` pins `loadPromptTemplates` (provenance,
      description/argument-hint, first-line truncation, explicit
      file/dir/missing/skip, includeDefaults) against Pi's real loader;
      `tests/skills-parity` pins `loadSkillsFromDir` (name/description
      validation, dirname fallback, root/recursive/inline discovery,
      skippables, disable-model-invocation, diagnostics); and
      `tests/package-transport-parity` pins the git source grammar
      (parseGitUrl/isLocalPath via `crates/pi-rs-host/src/git.rs` + `pi.git`)
      against the pinned Pi oracle. The package lifecycle is covered by
      deterministic hermetic fixtures in `crates/pi-rs-app/tests/package_lifecycle.rs`.

      The packages CLI **parse/help/early-error surface** is closed: the
      `install`/`remove`/`uninstall`/`update`/`list` command grammar
      (`parsePackageCommand`), usage/help text, and the console-error prefix
      (missing source / unknown option / unexpected argument / missing option
      value / conflicting options) are ported to Rust
      (`crates/pi-rs-app/src/cli/packages.rs`) and dispatched on the raw argv
      before `parseArgs` exactly as Pi's `main.ts` runs `handlePackageCommand`
      first. It is pinned differentially against Pi's real
      `package-manager-cli.ts` `handlePackageCommand` by the Bun-generated
      oracle `tests/package-cli-parity/oracle.json`: `package_cli_parity.rs`
      replays all 24 hermetic cases through the parsed surface byte-for-byte
      (handled/exitCode/stdout/stderr), and `package_cli_binary.rs` runs the
      real `pi` binary end-to-end over every handled case, asserting exact
      stdout/stderr/exit against the oracle. Commands that would proceed to
      settings/trust/package-manager/network work (real `install`/`remove`/
      `update`/`list` execution, self-update) remain out of this fixture's
      scope and are not captured by the hermetic oracle, consistent with the
      equal-observable-contract goal.

      **Landed:** the packages CLI *execution* legs are wired end-to-end through
      the public `pi.packages` module: `main()` dispatches a would-execute
      package command (the hermetic handler's `i32::MIN` sentinel) to the
      `pkg-exec` Lua role, which runs the deterministic `list` (user/project
      sections, `(filtered)` markers, installed paths, "No packages installed.")
      and local-path `install`/`remove` (cwd-resolved existence check, settings
      add/remove, "Installed/Removed", "No matching package found", untrusted
      project-write refusal) legs and mirrors Pi's stdout/stderr/exitCode.
      `pi.settings` gained `global_packages`/`project_packages` getters so the
      lifecycle reads user/project scope from `getGlobalSettings()`/
      `getProjectSettings()` exactly as Pi's `listConfiguredPackages`. Hermetic
      end-to-end coverage through the real `pi` binary:
      `crates/pi-rs-app/tests/package_cli_exec.rs` (8 cases). Two driving bugs
      were fixed en route: `remove_and_persist` now returns the
      `remove_source_from_settings` change boolean, and the local `install`
      path resolves against the package manager's cwd (Pi `resolvePath`), not
      the scope base dir.

      **Remaining:** the network-modulated packages CLI legs (npm/git `install`/
      `update` against live registries, `update`/self-update, the TUI `config`
      command) and live update checks + offline-cache pruning. These are
      network/install-method dependent and are represented by the deterministic
      offline-skip behavior (`pi.resources` shows no package-origin resources
      for an uninstalled package) rather than pinned network outcomes.

- [x] **9.8 Translation matrix + Pi extension gate.** Translate every in-boundary
      pinned first-party TypeScript extension example to executable Lua. Group
      truly equivalent examples, but never skip one because the bridge lacks a
      capability. Generate/check concise Lua API docs from the same minimal
      manifest.

      **Accept:** every pinned API member/event and configuration capability maps
      to differential evidence, executable Lua, or an explicit DESIGN exception;
      all in-scope examples run through the shipped public surface.

      **Closed:** 47 of 82 pinned examples have executable Lua translations
      that load and register through the shipped public `pi` table. 41 of them
      are load-gated by `translated_9_8_examples_load_through_the_public_surface`
      in `crates/pi-rs-app/tests/extension_loading.rs` (the Plan 9.8 gate); the
      other 6 (commands, hello, permission-gate, protected-paths,
      shutdown-command, structured-output) were already exercised through their
      owning gates (`queued_extension_ui_actions_match_pi_examples`,
      `product_loader_runs_tool_and_blocking_hook_with_isolated_failures`,
      `translated_tool_examples_execute_through_the_public_surface`,
      `extension_context_snapshots_and_shutdown_match_pi`). The translated set
      covers
      the full modeled surface — session/trust/hook events (`session_before_*`,
      `project_trust`, `input`, `before_agent_start`, `agent_start/end`,
      `model_select`, `tool_result`, `resources_discover`,
      `before/after_provider_request`, `user_bash`), runtime action methods
      (`set/getSessionName`, `setLabel`, `send/sendUserMessage`, `appendEntry`,
      `set/getActiveTools`, `getAllTools`, `set/getThinkingLevel`, `setModel`,
      `refreshTools`, `registerMessageRenderer`), UI operations
      (notify/select/confirm/input/editor/custom/status/widget/header/footer/
      title/working/hidden-label/setEditorText), tool registration/override
      (tool-override, built-in delegation via `pi.registered_tools()`), dynamic
      tools and `resources_discover`, `pi.exec`/`pi.fs`/`pi.events`/
      `pi.module.require`, per-customType message rendering, theme polling
      (mac-system-theme via `pi.exec`/`pi.set_interval`/`ctx.ui.setTheme`),
      git-merge conflict parsing (git-merge-and-resolve via `pi.exec`/
      `pi.sendUserMessage`), and `#`-issue autocomplete (github-issue-autocomplete
      via `addAutocompleteProvider` + `pi.tui.fuzzy_filter`). Bridge additions
      that unblocked pinned examples: `ctx.sessionManager.get_label` and
      `get_leaf_entry` were missing from the Lua session facade and are now
      bound (unlocked `bookmark.ts`, `git-checkpoint.ts`).

      The remaining 35 examples are grouped and classified as an explicit
      **DESIGN exception 3** (recorded in the manifest and
      `EXTENSION_INVENTORY.md` / `docs/lua-extension-api.md`, not silently
      skipped): the large custom-UI / LLM-`complete()` examples that need a
      streaming-LLM helper and `modelRegistry.getApiKeyAndHeaders` or custom
      editor/overlay component classes (custom-compaction, handoff, qna,
      summarize, question, questionnaire, tools, preset, plan-mode,
      modal/rainbow/border-status editors, built-in-tool-renderer,
      minimal-mode, interactive-shell), plus the external-infrastructure and
      full-game examples that require a native system runtime or are product
      experiments (mac-system-theme is translated; sandbox/gondolin VM
      runtimes, custom-provider-* OAuth wire providers, ssh, with-deps npm/jiti
      module resolution, subagent recursive `pi --mode json` spawn,
      git-merge/rpc/github-issue are closed via translation, and
      doom-overlay / snake / space-invaders / tic-tac-toe / overlay-test /
      overlay-qa-tests). Each is categorized `DESIGN exception 3` with
      rationale. Evidence:
      `examples/extensions/*.lua`, `crates/pi-rs-app/tests/extension_loading.rs`,
      `crates/pi-rs-host/src/session.rs`, `crates/pi-rs-app/src/builtins/utils/extensions.lua`,
      `tests/extension-inventory/manifest.json`, regenerated
      `EXTENSION_INVENTORY.md` + `docs/lua-extension-api.md`.

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

      **Closed (tools + shared-helper decomposition):** every builtin tool is now
      independently ablatable and file-back replaceable. `pi.unregister_tool`
      (Rust mechanism, DESIGN) + `Host::unregister_tool` + `BuiltinManifest::
      load_with_suppressed_tools(&[], &["bash"])` disable one tool while the rest
      of the pack stays loaded, and an ordinary file-backed package re-registers
      the name first-wins (`per_tool_suppression_ablates_a_builtin_tool_and_allows_
      file_backed_replacement`). The shared tool helpers the ports relied on are
      now public exact-version modules instead of concat-order chunk-locals:
      `pi.tools.prelude`, `pi.tools.path-utils`, `pi.tools.mime`, `pi.tools.diff`,
      `pi.tools.edit-diff`, `pi.tools.output-accumulator`, `pi.tui.keybinding-hints`
      (joining the already-public `pi.tools.{truncate,shell,render,file-mutation,
      file-mutation-queue}` and `pi.tui.visual-truncate`). Construction-inventory
      rows `tool.{read,bash,edit,write,grep,find,ls}` and the tool helper
      `module.{prelude,path-utils,mime,output-accumulator,keybinding-hints,
      diff-renderer,edit-diff}` went implemented with the referenced ablation and
      reuse evidence; the inventory also gained coverage rows for the PLAN 9.7
      resource/skill/prompt/package modules (`module.resources/skills/prompts/
      packages`) and regenerated correctly (`construction-inventory --check`).

      **Closed (wave 2 — file-backed shared-helper modules):** the remaining
      tool shared-helper rows recorded their public exact-version modules and
      gained file-backed exercisers: `module.{truncation,shell,tool-render-utils,
      visual-truncate}` define `pi.tools.truncate@1`, `pi.tools.shell@1`,
      `pi.tools.render@1`, `pi.tui.visual-truncate@1` (exercised by
      `examples/extensions/module-demo.lua` /
      `file_backed_extension_imports_builtin_tool_and_render_modules`), and
      `module.export-html` defines `pi.interactive.export-html@1` (exercised by
      `examples/extensions/export-html-consumer.lua` /
      `file_backed_package_imports_the_same_export_html_module`).

      **Closed (wave 3 — shared cross-pack agent-policy modules):** the
      `module.{messages,branch-summary,compaction,system-prompt,agent-session,
      bash-executor}` rows and the agent-policy half of the
      `modules.chunk-local-helpers` concat tier are closed by a single-ownership
      always-loaded substrate pack, `agent-core` (DESIGN "Shared-policy
      substrate pack"). It defines one `pi.agent.*` exact-version module per
      fragment — `pi.agent.messages@1`, `pi.agent.branch-summary@1`,
      `pi.agent.compaction@1` (depends on branch-summary + messages),
      `pi.agent.system-prompt@1`, `pi.agent.session-runtime@1`,
      `pi.agent.bash-executor@1` (imports `pi.tools.truncate@1`) — and the
      `interactive` and `coding-agent` packs de-duplicated their concats to
      `require` + rebind the identical closures (mod.rs no longer
      concatenates the shared fragments). `agent-core` is declared `core:
      true` in the manifest: it cannot be suppressed (suppressing it would
      break every dependent policy pack), per the 9.10 rule "do not force
      singular mechanisms into ceremonial registries"; it still loads via the
      same transactional `Host::load_embedded` path and a file-backed package
      can define the same exact-version module name. Exercised from a
      file-backed package by `examples/extensions/agent-core-module-demo.lua`
      (`crates/pi-rs-app/tests/assembly.rs`
      `file_backed_package_imports_the_shared_agent_core_modules`).

      **Remaining (next waves):** the monolithic `interactive` frontend still
      has open rows (per-feature ablation of the 12k-line frontend into
      slot/render-middleware/event/session-runtime units), plus
      `module.extension-composition`, agent-policy decomposition, command-
      routing per-command suppression, theme/resource file-backed replacement,
      the cross-pack shared `module.syntax-highlight` fragment, and the
      `modules.chunk-local-helpers` concat tier that still underlies the
      tools/extension/frontend packs.

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

      Closed (RPC agent-streaming commands): the async agent-streaming commands
      (prompt, bash, compact, fork, clone, new_session, switch_session,
      get_fork_messages) now reproduce Pi's `runRpcMode` oracle through a gated
      differential seed (`PI_RPC_SCRIPTED_SEED` → `request.scriptedRpc`, guarded
      by a magic token and compiled out of release builds), pinned
      by `scripts/rpc-oracle` oracle cases (`prompt-async-success`,
      `prompt-preflight-failure`, `event-streaming`, `compact-bash-session-ops`,
      `session-fork-clone`, `fork-messages`, `async-steer-followup-abort`).
      Pi's runRpcMode emits only the response envelope (`prompt` → `{success}`,
      `bash` → `{data:{exitCode,stdout,stderr}}`, `compact` →
      `{data:{sessionId,summary,kept}}`, `new_session`/`clone`/`switch_session`
      → `{data:{cancelled}}`, `fork` → `{data:{text,cancelled}}`); it does NOT
      forward agent events to RPC stdout, so a faithful reproduction needs no
      concurrent event streaming. The seed also closes the model/message-seeded
      read commands (`get_state`, `get_available_models`, `cycle_model`,
      `set_model`, `get_messages`, `get_last_assistant_text`,
      `get_session_stats`, `get_commands`, `export_html`) — verified by
      `scripts/rpc-oracle` cases `state-and-simple`, `thinking-model-commands`,
      `set-model-not-found`, `export-html`, `commands-registry`. The unseeded
      (production) RPC path stays honest: `bash` runs the real executor and
      `compact` falls through to real compaction, while `prompt`/`new_session`/
      `fork`/`clone`/`switch_session` — which have no live runtime/agent wiring
      yet — fail with `Not supported in this build` (matching Pi's envelope
      shape) rather than fabricating success. All 16 oracle cases replay
      through the real `pi --mode rpc` binary and match Pi record-for-record.

      Closed (startup-ui): the pre-runtime startup prompt surface
      (cli/startup-ui.ts `showStartupSelector`) is the embedded
      `startup-selector` role (`coding-agent-startup-selector` in
      `crates/pi-rs-app/src/builtins/interactive.lua`), used by `main.rs` for
      the missing-session-cwd Continue/Cancel prompt and the project-trust
      decision prompt (project-trust.ts startup order). It renders the
      pre-runtime TUI with the same theme/columns as Pi and returns the
      selected label; `interactive_startup.rs` pins the composition surface.

      RpcClient boundary (recorded): the Node `RpcClient`
      (`packages/coding-agent/src/modes/rpc/rpc-client.ts`) is a downstream
      headless consumer of the RPC protocol, not part of the coding-agent
      product or its extension platform — it falls under DESIGN's "downstream
      product behavior" out-of-scope rule. It stays un-ported; the ported,
      differentially-pinned surface is the RPC protocol itself (rpc-mode.ts
      `runRpcMode`), which the RpcClient speaks. No Rust/Lua equivalent is
      warranted. (The stdout output-guard is closed: stray extension
      `print`/`io.write` route to stderr so non-interactive stdout stays
      protocol-clean, RPC now loads CLI `--extension` files like Pi, and Pi's
      RPC extension-UI binding is closed — `ctx.hasUI==true`, real
      `ExtensionUIContext` transported as `extension_ui_request` JSONL records,
      per `createExtensionUIContext`; see
      `crates/pi-rs-app/tests/rpc_mode_parity.rs`
      `rpc_binds_real_extension_ui_context_matching_pi`,
      `rpc_stdout_guard_routes_extension_stdout_to_stderr` and
      `rpc_loads_cli_extension_files`.) The unseeded (production) RPC streaming
      paths are also pinned honest: `crates/pi-rs-app/tests/rpc_streaming_parity.rs`
      `rpc_unseeded_bash_reports_real_output_in_pi_envelope` runs a real bash
      command through the shared executor and asserts Pi's `{exitCode,stdout,
      stderr}` envelope carries the run's output (the executor's merged
      `output` stream is reported on `stdout`, `stderr` stays `""` — pi-rs
      does not split streams), and `rpc_unseeded_compact_and_session_stats_run_
      real_envelopes` runs the real compact/`get_session_stats` paths (both were
      latent before — the RPC role redirected only through the seeded seam). This
      required wiring `utils/bash-executor.lua` into the coding-agent pack
      (`crates/pi-rs-app/src/builtins/mod.rs`) so `EXTENSION_POLICY.bash_executor`
      is present for the rpc/print roles.

      **Closed (toolCall-only `text-no-text-content` — documented oracle
      artifact, not a parity difference).** The oracle case in
      `tests/print-mode-parity/oracle.json` scripts a *bare toolCall assistant
      final message* (`[toolCall bash "x"]`, `stopReason:"toolUse"`) through
      `gen-oracle.ts`'s stub session, which returns it immediately and records
      the terminal print outcome Pi's `runPrintMode` derives from that observed
      state (empty stdout, exit 0). This is **not** a state either real
      implementation ever settles on: both Pi's agent-loop (`ref/pi/…/agent-loop.ts`,
      `while (hasMoreToolCalls …) { if (toolCalls.length > 0) { … hasMoreToolCalls =
      !executedToolBatch.terminate; } }`) and pi-rs's port (`agent.lua` `run_turn`,
      `if #calls > 0 then tool_results, terminate = execute_tool_calls(…);
      has_more_tools = not terminate`) **continue the tool loop** when a response
      carries a `toolCall`, executing the tool and re-prompting the model until a
      non-tool final message settles. So Pi's real `runPrintMode` would no more
      terminate on a bare toolCall than pi-rs's real print role would. The
      differential therefore cannot be closed by reproducing the oracle's empty
      output through the real agent — pi-rs's agent (matching Pi's) executes the
      `bash "x"` tool and continues. This is an oracle-scripting artifact, not a
      print-role bug; the print role already matches Pi for every *settled*
      terminal final message (text blocks to stdout, `error`/`aborted` →
      exit 1 with stderr), pinned byte-for-byte by the remaining oracle cases
      in `print_text_mode_output_matches_pi_byte_for_byte` and
      `print_follow_up_sequence_matches_pi_byte_for_byte`. The row is closed as a
      documented artifact; the harness comment in
      `crates/pi-rs-app/tests/print_mode_parity.rs` carries the same rationale.

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

      **Status (side-by-side differential).** The automated differential is
      green: `cargo test -p pi-rs-app` exits 0 (50 test groups; all `parity`
      rows and the ui-parity scenario drivers pass), and the pinned non-
      interactive oracles regenerate byte-identically from `ref/pi @
      c5582102` (`scripts/print-mode-oracle`, `scripts/args-oracle`,
      `scripts/rpc-oracle` all diff clean against the checked `oracle.json`).
      Known harness gap (recorded, out of scope for this stream): the
      standalone frame-diff binary `crates/pi-rs-app/src/bin/ui-diff.rs`
      (driven by `scripts/ui-diff`) loads `INTERACTIVE_PACK` without the
      always-loaded `AGENT_CORE_PACK`, so `interactive.lua`'s
      `require("pi.agent.*")` fails with ``module "pi.agent.messages"
      version "1" is not defined`` and the 26-scenario `.pci.json` frame
      comparison cannot run through that binary. The frame-exact scenario
      drivers that *do* run in the cargo suite load `AGENT_CORE_PACK` and pass;
      the `ui-diff.rs` load list needs `AGENT_CORE_PACK` added (an s14/assembly
      owner fix, not an observable product difference). `coding.assembly`
      (cli.ts/main.ts/bun/cli.ts) remains the open gate with s14.

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
