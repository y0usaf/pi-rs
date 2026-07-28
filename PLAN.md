# pi-rs — execution plan

`main` builds **pi-rs**: a minimal, high-performance Rust coding harness whose
shipped product looks and feels like Pi but is authored entirely through its
public Lua surface. The former faithful port is preserved on
`pi-rust-rewrite`; it is a source of proven mechanisms and focused reference
observations, not the product specification for `main`.

The first unchecked dependency-ready item is next. Items marked **serial** own
shared contracts or hot files and must land before a dependent wave starts.
Items sharing a **wave** may be assigned by `/orchestrate` only when their exact
paths are disjoint on the integration base. `PLAN.md` is updated by the
integrator, never by parallel workers unless explicitly assigned.

## Product contract

### What pi-rs preserves

Pi is the experience reference for a deliberately bounded set of canonical
interactive states:

- transcript rhythm, spacing, color, wrapping, and tool presentation;
- editor behavior, completion, keybindings, paste, and external-editor flow;
- streaming/thinking presentation and cancellation;
- steering and follow-up queues;
- selectors, dialogs, notifications, status, header, and footer;
- the restrained default coding workflow.

Canonical terminal grids and input traces may be cell-exact. Behavior outside
that checked experience set belongs to pi-rs and need not reproduce Pi.

### What pi-rs is

- Installed executable: `pi`.
- Configuration and product policy: Lua 5.4.
- Rust: generic mechanisms only.
- Shipped defaults: ordinary Lua packages loaded through the same API as
  file-backed user packages.
- Configuration root: `${XDG_CONFIG_HOME:-~/.config}/pi`.
- Data root: `${XDG_DATA_HOME:-~/.local/share}/pi`.
- State root: `${XDG_STATE_HOME:-~/.local/state}/pi`.
- Cache root: `${XDG_CACHE_HOME:-~/.cache}/pi`.
- Legacy `~/.pi/agent` resources are read-only fallbacks when the corresponding
  XDG resource is absent. pi-rs never writes, deletes, or silently migrates
  legacy files.
- Provider/auth scope is intentionally broad: retain full pinned-Pi parity for
  supported provider protocols, model catalog behavior, API-key resolution,
  and subscription OAuth flows. This is a subsystem compatibility promise, not
  whole-product parity.

### Mechanism/policy boundary

Rust owns:

- the Lua VM, package/module loading, watchdogs, and source-neutral capability
  checks;
- immutable snapshot creation, action validation/application, async effect
  execution, cancellation, and scoped resource disposal;
- terminal byte decoding, cell/display primitives, layout/clipping primitives,
  and differential ANSI presentation;
- HTTP/SSE/WebSocket, process, filesystem, timer, crypto, and image primitives;
- provider wire protocols and authentication engines;
- a generic durable append-only record store and atomic filesystem mechanics.

Lua owns the complete product:

- application, agent, frontend, and session state machines;
- tool-loop, retry, queue, compaction, and context policy;
- editor behavior, keymaps, TUI composition, transcript rendering, and every
  visible default;
- tools, commands, themes, configuration, provider selection, resource
  discovery, and session semantics.

Lua receives immutable snapshots/read-only handles and returns or queues typed
actions. It never receives mutable host state. Rust may batch, lay out, clip,
and diff Lua-authored display structures; it may not decide product appearance
or workflow.

### Extension model

Extensibility is broad capability through a small number of stable seams, not a
registry for every helper:

- coarse replaceable roots: application, agent, frontend, session;
- composable declarations: tools, commands, providers, events, renderers, UI
  slots, themes, and keymaps;
- ordinary versioned Lua modules for private and shared helpers;
- one declaration mechanism per kind;
- no capability, lifecycle, priority, or module available only to embedded
  builtins.

A root or declaration is independently replaceable. Private implementation
functions and inert package resources need not become ceremonial units.

### Session model

Persistent sessions are optional Lua policy over a generic Rust record store.
The shipped session package defines creation, naming, branch/tree semantics,
context reconstruction, compaction records, selection, and retention. It uses
public snapshots/actions only and can be disabled or replaced by a file-backed
package; without it, the application remains useful with an ephemeral
conversation. Rust guarantees durable append, atomicity, locking, iteration,
and cancellation, but knows no Pi session workflow.

### Explicit non-goals

- whole-product behavioral or data compatibility with Pi;
- compatibility with Pi's TypeScript extension API or npm package runtime;
- exact Pi errors, CLI breadth, request ordering outside provider/auth, or
  historical edge cases;
- reproducing every Pi mode, export path, package-manager behavior, easter egg,
  or external extension;
- making every Lua helper independently registered;
- claiming performance from implementation language alone.

JSON/RPC modes, HTML export, package registries, and other omitted features may
return only after an independent use case and must be ordinary Lua packages or
generic mechanisms.

## Execution rules

1. **Contract before parallelism.** Do not launch a wave against an unsettled
   root API, action vocabulary, display schema, or storage contract.
2. **Small waves.** Prefer 2–4 path-disjoint deliverables. A shared hot file
   makes the frontier serial.
3. **No speculative compatibility.** `ref/pi` and `pi-rust-rewrite` may answer a
   focused visual/provider question; they do not authorize porting adjacent
   behavior.
4. **Evidence earns its cost.** Each permanent test names one distinct contract:
   mechanism invariant, public Lua capability, canonical experience, provider/
   auth parity, or performance budget.
5. **Performance is measured.** Track release startup, idle RSS, input-to-frame
   latency, sustained render cost, Lua dispatch/effect overhead, and binary
   size. Avoid per-cell/per-byte Lua crossings and unbounded snapshot copies.
6. **Bare core boots.** With no builtins, config, or extensions, `pi` can load a
   file-backed Lua application, accept input, render, run an effect, and exit
   cleanly. A missing/broken product package produces a useful diagnostic.
7. **Nix is authoritative.** Completion claims use `nix build`/`nix flake
   check`; direct Cargo is an iteration aid except for sanctioned fmt/clippy.
8. **Git is the attic.** Delete superseded code and evidence from `main`; recover
   history from `pi-rust-rewrite` rather than retaining migration layers.
9. **Usability is a ratchet.** The bare kernel and shipped product are separate
   acceptance targets. Once the default product returns in 3.6, every later
   integration base must keep `nix run` input-ready; ablation runs through the
   dedicated bare/file-backed target and never replaces the default artifact.

## 0 — Reset the contract and coordination tools

- [x] **0.1 — Replace the legacy parity contract** (**serial**).

  Rewrite `DESIGN.md` around this product contract, including a doctrine table,
  explicit hot-path/mechanism decisions, the XDG/legacy policy, provider/auth
  subsystem parity, configurable sessions, and measurable performance goals.
  Update `README.md` so it no longer promises a faithful port. Rewrite
  `.pi/skills/{next,parallel-plan,orchestrate}/SKILL.md` to use this plan,
  preserve path/base locking, and stop treating all of Pi as the oracle.

  **Own:** `DESIGN.md`, `README.md`, `.pi/skills/**` only.

  **Accept:** the documents agree; the first-open-item and wave semantics are
  unambiguous; workers are directed to Pi only for named experience or
  provider/auth evidence; no implementation work begins under the old
  contract.

  **Landed:** `1a5c66b` replaces the faithful-port promise with the bounded
  experience + exhaustive provider/auth subsystem contract and locks the
  coordination skills to serial/frontier/wave, exact-base, and path ownership.

- [x] **0.2 — Establish compact experience and performance baselines**
  (**serial after 0.1**).

  Select a small canonical set covering startup, prompt editing, streaming,
  thinking, one tool call/result, queueing, cancellation, selector/dialog, and
  session resume. Convert only those observations into a compact versioned
  grid/input format. Add a reproducible release-mode benchmark harness for
  startup, idle RSS, input-to-frame latency, render throughput, Lua dispatch,
  and effect round trips. Record explicit initial budgets in `DESIGN.md` from
  measured data rather than aspiration.

  **Own:** new `tests/experience/**`, new `tests/performance/**`, focused harness
  code, and the budget section of `DESIGN.md` as an explicit shared exception.

  **Accept:** fixtures are reviewable and byte-idempotent; negative controls
  identify the first cell/input mismatch; benchmarks emit stable machine-
  readable results; normal checks do not execute Node/TypeScript Pi.

  **Landed:** `1bb2758` adds 6 compact journeys / 20 cell-exact checkpoints and
  offline mismatch/idempotence checks plus the release benchmark; `cbb0ab4`
  records measured reference baselines and budgets. Integrated release results
  remained within every budget; workspace Cargo/Nix and release-package checks
  passed.

- [x] **0.3 — Remove faithful-port scaffolding** (**serial after 0.2**).

  Delete exhaustive parity snapshots, external-extension fixtures, generated
  construction/final audits, oracle wrappers, stale parity documents, and
  checks whose contracts are not retained by 0.2 or provider/auth parity.
  Reconcile the flake and source filters once. Keep focused Rust mechanism tests
  and provider/auth wire fixtures.

  **Own:** legacy `tests/**`, `scripts/**`, parity inventory documents,
  `flake.nix`, and generated-check wiring. Preserve the new 0.2 paths.

  **Accept:** tracked size drops substantially; every remaining suite states its
  unique owner; `rg` finds no active whole-product parity promise; the flake is
  green from a clean tree.

  **Landed:** `5d79950` removes 219.9 MB (96.55%) of legacy evidence, records
  retained-suite ownership, and reconciles the flake. Integrated workspace tests
  and clean-tree `nix flake check` pass.


## 1 — Cut the bare mechanism kernel

- [x] **1.1 — Define the kernel transaction and source-neutral package model**
  (**serial**; depends on 0.3).

  Replace compatibility-shaped host APIs with one bounded dispatch transaction:
  immutable event/context snapshot in, validated action/effect batch out. Define
  generation-safe read handles, cancellation, watchdog behavior, scoped
  resources, errors, and deterministic action ordering. Define versioned Lua
  modules plus coarse root and composable declaration registries. Embedded and
  file-backed packages must enter the identical loader transaction.

  Split central host code into ownership-friendly modules before adding more
  bindings; `api.rs` must not remain the universal hot file.

  **Own:** `crates/pi-rs-host/src/**`, host tests, and host crate manifest.

  **Accept:** file-backed tests prove equal capability and lifecycle; stale
  handles fail; busy loops time out; actions apply only after dispatch; failed
  package loads publish nothing; root/declaration conflicts are deterministic.

  **Landed:** `7ce1736` establishes the versioned bounded transaction,
  source-neutral package scopes, stale-handle/watchdog/cancellation/disposal
  behavior, deterministic roots/declarations/modules, and 13 focused invariant
  tests. `d6618ac` removes the inherited `api.rs` hotspot, splits ownership into
  focused modules, routes retained adapters through canonical package state,
  and proves scope-atomic rollback across every registration family with 5
  additional tests. Integrated host/workspace tests and `nix flake check` pass.

- [x] **1.2 — Make `pi` a thin generic launcher with zero builtins**
  (**serial**; depends on 1.1).

  Reduce `pi-rs-app` to CLI parsing, XDG/legacy root discovery, host creation,
  package graph loading, and generic application-root selection. Remove product-
  named Rust branches. Builtins are optional input, not linked assumptions.

  **Own:** `crates/pi-rs-app/src/**` excluding future builtin assets, app tests,
  and app crate manifest. Root workspace/flake edits are integrator-owned.

  **Accept:** zero-pack `pi` loads and runs an ordinary file-backed application;
  missing/broken packages diagnose cleanly; no Rust identifier names a shipped
  command, screen, tool, or session workflow.

  **Landed:** `c512181` removes the inherited embedded product and reduces `pi`
  to ordered file-package loading plus generic `application` root dispatch, with
  binary-level ablation, ordering, conflict, and failure diagnostics. `e78a8d8`
  makes the installed Nix smoke exercise the same ordinary file-backed path;
  `86acfca` and `d5d5f6a` close lint and lockfile reconciliation. Integrated app/
  workspace tests, `nix flake check`, and release `nix build .#pi-rs` pass.

- [x] **1.3 — Implement deterministic XDG roots and read-only legacy fallback**
  (**serial**; depends on 1.2).

  Expose canonical config/data/state/cache paths as immutable startup data.
  Resolve `~/.pi/agent` per resource only when its XDG counterpart is absent.
  Writes always target XDG; fallback files never merge ambiguously and are never
  modified. Cover environment overrides, missing HOME/XDG values, permissions,
  symlinks, and explicit import diagnostics.

  **Own:** focused app/host path modules and tests. Do not add product resource
  loading policy in Rust.

  **Accept:** a compact matrix proves precedence and no-write behavior for
  config, credentials, sessions, packages, and cache resources.

  **Landed:** `06ab41f` adds deterministic validated XDG/HOME roots, immutable
  file-backed startup visibility, lstat-style per-resource precedence, canonical-
  only destinations, and explicit non-mutating import reports. The five-resource
  matrix covers overrides/defaults, absent/invalid environments, malformed and
  inaccessible entries, symlinks, fallback provenance, and byte/metadata-stable
  legacy resources. Integrated app/workspace tests, formatting, `nix flake check`,
  and release `nix build .#pi-rs` pass.

## 2 — Harden reusable Rust mechanisms

After 1.3, `/orchestrate` may run **Wave M**. Workers may improve internals and
public mechanism contracts already present on the base; they may not invent Lua
product policy. Root manifests and central binding indexes are reconciled by the
integrator after the wave.

- [x] **2.1 — Terminal/display mechanism** (**Wave M**, path owner:
  `crates/pi-rs-tui/**`; depends on 1.3).

  Retain input decoding, Unicode cells, width/wrapping primitives, clipping,
  focus primitives, image capability, and differential ANSI presentation.
  Remove Pi-specific component policy. Define a batched retained display tree or
  display-list boundary suitable for Lua-authored UI without per-cell callbacks.

  **Accept:** mechanism tests cover Unicode/wide cells, resize, cursor, clipping,
  minimal diffs, and malformed input; benchmark budgets from 0.2 pass.

  **Landed:** `a2e32b4` adds the versioned bounded retained display tree,
  transactional validation, iterative layout/clipping, Unicode cells, stable
  identities, focus/cursor metadata, minimal ANSI diffs, and bounded raw-input
  decoding. `bc43cc4` and `5c0db47` migrate the narrow host/performance seams and
  delete inherited Pi-specific editor, markdown, selector, settings, loader,
  component, TUI policy, parity tests, and stale examples. Integrated focused,
  workspace, release performance, and Nix checks pass.

- [x] **2.2 — Async OS/effect mechanism** (**Wave M**, path owner:
  `crates/pi-rs-host/src/effects/**` plus focused new tests; depends on 1.3).

  Consolidate abort-aware filesystem, process-tree, HTTP streaming, timer,
  clipboard, and crypto effects behind typed queued requests. Every resource has
  timeout, cancellation, reload, and shutdown behavior. No effect retains mutable
  product state.

  **Accept:** file-backed Lua exercisers cover each effect; leak tests prove no
  process/task/socket survives disposal; backpressure is bounded.

  **Landed:** `57c7aec`, `3ec66f1`, and `7e3a82f` add one scope-owned typed
  effect queue for filesystem, process trees, HTTP streams, timers, clipboard,
  and crypto, with bounded channels/output plus cancellation, timeout, reload,
  disposal, and shutdown cleanup. File-backed coverage and process/task/socket
  leak tests pass; `0a83812` reconciles the dependency lock.

- [x] **2.3 — Generic durable record store** (**Wave M**, path owner:
  `crates/pi-rs-session/**`; depends on 1.3).

  Replace Pi-session semantics with a generic versioned append-only JSON-value
  log: create/open/list, atomic append, read cursors, branch/file copy primitive,
  locking, corruption diagnostics, and cancellation. It stores policy records
  without interpreting conversation roles, compaction, names, or tree meaning.

  **Accept:** crash/partial-write, concurrent-open, lock, corruption, iteration,
  copy, and XDG-path tests pass; a file-backed Lua package uses it without private
  methods.

  **Landed:** `bababc1` replaces the session crate with an opaque, checksummed
  record log with synced append/copy, file locks, bounded cursors, diagnostics,
  cancellation, and explicit destinations; `5889eee` exposes it to an ordinary
  file-backed Lua package. `901c6ec` replaces the stale product-session example
  with a generic caller-owned record consumer. Focused and integrated tests pass.

- [x] **2.4 — Provider transport and auth mechanism preservation** (**Wave M**,
  path owners: `crates/pi-rs-ai{,-types,-auth}/**`; depends on 1.3).

  Preserve and simplify shared transport, protocol conversion, streaming,
  cancellation, model catalog, credential storage, PKCE, and device-code engines.
  Remove dependencies on the old product host while retaining pinned provider/
  auth parity fixtures.

  **Accept:** focused wire replays remain deterministic; secrets are redacted;
  credential writes use XDG only while legacy credentials are fallback-readable;
  shared transport/retry/SSE machinery has one implementation.

  **Landed:** `d493178` restores the pinned 35-provider/969-model catalog,
  fail-closed dispatch/auth inventory, pooled transport, explicit XDG credential
  storage with read-only legacy fallback, cancellable OAuth flows, and broad
  redaction evidence while retaining deterministic protocol/auth replays.
  `4939ab8`, `d75d8ff`, and `7eb1146` close review with all-length known-secret
  redaction, a deterministic bounded command cache, and crash-safe OS-released
  credential locks. Focused catalog/protocol/auth and integrated Nix checks pass.

## 3 — Recover usability through a public vertical slice

The default artifact is currently the deliberately ablated launcher. Correct
that sequencing error without restoring the inherited compatibility product:
first prove the smallest coding journey from ordinary files, then ship the same
packages as defaults. Do not design the complete Lua API in advance of a product
consumer.

- [x] **3.1 — Expose the minimum public coding spine** (**serial**; depends on
  all Wave M items).

  Define compact versioned modules and snapshot/action contracts only for the
  first useful journey: package/default-manifest loading; application, agent, and
  frontend roots; terminal input and retained display submission; provider/model
  lookup and bounded streaming; cancellation; and the filesystem/process effects
  needed by the first tools. Split bindings by mechanism and keep shared indexes
  under one owner. Distribution may select a default manifest, but embedded and
  file-backed sources must enter the canonical package transaction with identical
  capabilities.
  This slice may delete or replace inherited compatibility bindings, but builtin
  work may not consume them, private host methods, or synthetic-source escapes.

  Separate the two launch targets now: the raw `pi-core` package has no selected
  policy and can run an explicitly supplied ordinary application; the default
  `pi` distribution target will select shipped packages in 3.6. A no-package raw
  invocation prints useful launcher/package guidance and exits cleanly instead of
  treating intentional absence as a product crash.

  **Own:** `crates/pi-rs-host/src/bindings/**`, focused host binding tests,
  `crates/pi-rs-app/src/**`, app launcher tests, concise API docs for this slice,
  and the serial package/app/check split in `flake.nix`.

  **Accept:** ordinary file-backed packages can receive terminal input, submit a
  retained frame, resolve and stream a fixture model, execute/cancel bounded
  effects, and shut down; snapshots/actions remain the only state boundary;
  synthetic embedded identity grants no capability; Nix separately checks the
  clean raw launcher and its file-backed application path.

  **Integrated slice:** `5d5a734`, `a5eb81e`, and `767e2a2` add the focused
  versioned roots/terminal/models/effects surface, generic manifest selection,
  raw no-package guidance, separate `pi-core`/`pi-rs` outputs, and file-backed
  end-to-end/source-neutral evidence. `521daf8` removes the inherited broad
  compatibility surface, Rust agent product, stale extension packs, and
  redundant policy tests while retaining compact kernel/effect/display/model
  invariants. Focused/workspace tests, `nix flake check`, and both Nix package
  builds pass. The worker's release run passed all budgets, but three integrated
  reruns under load did not. A fourth, quiescence-gated integrated run from
  `6779746` passed 34/35 budget checks: input p50/p95 measured 15.639/23.955 µs,
  and effect budgets passed. No implementation defect is identified.
  **Closed 2026-07-28:** three consecutive contention-controlled release runs at
  `0872ab2` (`reference-v1.json`; cores 16–23 pinned via `taskset`; background load
  ≈3 from unrelated desktop/agent processes) each passed all 35 budget checks —
  input p50/p95 measured 10.831/16.131, 10.936/15.69, and 11.54/15.92 µs against
  the 15/25 µs budget — with startup, RSS, render, Lua dispatch/snapshot, and
  effect budgets passing at wide margins. The earlier misses were CPU-contention
  scheduling jitter on the cross-process input pipe: variance explained by the
  measurement environment, and pinned reruns reproduce the pass consistently.
  Closure evidence: a credible passing integrated release measurement; diagnostic
  artifacts `/tmp/perf-run{1,2,3}.json` (uncommitted per
  `tests/performance/README.md`).

- [ ] **3.2 — Prove a file-backed coding walking skeleton** (**serial**; depends
  on 3.1).

  The launcher is currently one-shot: it dispatches a single synthetic startup
  event, serializes the resulting batch as JSON, and exits. No interactive
  product can exist on that. This gate builds the generic product loop that
  joins the proven mechanisms: terminal bytes → bounded input batches → root
  dispatch → action/effect settlement → retained display diff → ANSI frames,
  repeated until a shutdown action. Scheduling, wakeup, effect settlement, and
  frame presentation are Rust mechanism; the meaning of every action stays Lua
  policy. No private APIs, no privileged scheduler, no mutable host escapes.

  Alongside the loop, build small ordinary packages under `examples/` that
  establish the intended module/root contracts before builtin work: an
  application coordinates an ephemeral agent and frontend; the frontend accepts
  one prompt and renders incremental output; the agent streams a deterministic
  fixture provider and can invoke representative filesystem/process effects.
  Include cancellation and a clear missing-model/auth state. Keep editor and
  transcript behavior deliberately minimal.

  Independently replace the application, agent, and frontend roots and compose
  one event middleware plus one render middleware. Prove priority/conflict
  rules, module versions, watchdog isolation, rollback, and scope cleanup on
  this narrow surface.

  **Own:** `crates/pi-rs-app/src/**`, any new public host seam the loop
  requires (serial with the root/binding indexes), `examples/**`, and the
  PTY-driven acceptance tests.

  **Accept:** a PTY harness drives `nix run .#pi-core -- --package ...`: typed
  prompt → incremental rendered frames from the fixture provider → a
  representative tool effect → cancellation of in-flight work → return to an
  input-ready frame → clean exit with all scopes disposed. Missing model/auth
  diagnoses usefully without a private API. Deleting every builtin asset does
  not affect the test.

  **Falsifier:** this gate is the architecture's proof, and it is timeboxed.
  If the loop cannot be composed from the public snapshot/action/effect
  contracts without privileged escapes, stop: amend `DESIGN.md` and this plan
  before any Wave U work begins.

After 3.2, `/orchestrate` may run **Wave U**. Workers own disjoint package trees;
none may edit the default manifest, root binding indexes, or `flake.nix`.

- [ ] **3.3 — Minimal ephemeral agent package** (**Wave U**, path owner:
  `crates/pi-rs-builtins/agent/**`; depends on 3.2).

  Implement the Lua agent reducer needed for a useful coding turn: prompt and
  provider stream consumption, sequential/parallel tool settlement, cancellation,
  bounded retry, and steering/follow-up queues. It has no persistence dependency
  and uses only the public modules proven by 3.2.

  **Accept:** deterministic fixtures cover text, tool use, cancellation, retry,
  malformed provider events, steering, and follow-up; replacing the agent root
  changes transition policy without a frontend fork.

- [ ] **3.4 — Minimal application/frontend package** (**Wave U**, path owner:
  `crates/pi-rs-builtins/frontend/**`; depends on 3.2).

  Implement the Lua application/frontend roots, retained component tree, basic
  multiline input, transcript rows, focus/input routing, resize, invalidation,
  streaming updates, missing-auth/model guidance, and graceful shutdown. Keep
  editor/transcript/chrome modules separate even where behavior is initially
  sparse; Rust receives display/effect actions only.

  **Accept:** fixture journeys reach an input-ready frame, submit a prompt, show
  incremental assistant/tool output, cancel, and exit; file-backed roots can
  replace the application or frontend and wrap the render middleware.

- [ ] **3.5 — Minimal core-tool package** (**Wave U**, path owner:
  `crates/pi-rs-builtins/tools/**`; depends on 3.2).

  Ship `read`, `write`, `edit`, and `bash` as Lua declarations over public
  filesystem/process/diff effects. Tool execution, truncation, cancellation, and
  file-mutation serialization are policy in this package. Do not restore inherited
  utility mega-chunks or add a privileged builtin executor.

  **Accept:** each tool is independently suppressible/replaceable from disk;
  concurrent mutations are safe; path errors, bounded output, process-tree
  cancellation, and representative render data are covered.

- [ ] **3.6 — Assemble defaults and restore `nix run`** (**serial after Wave U**).

  Add the dedicated builtins layer and one declarative distribution manifest for
  the agent, frontend/application, and tool packages. Integrate package indexes
  once; do not concatenate sources or grant embedded-only modules. Make the
  default Nix package/app select this manifest while retaining `pi-core` as the
  explicit zero-builtin target.

  **Own:** builtin crate/package manifest and indexes, workspace manifest,
  `crates/pi-rs-app` distribution integration, `flake.nix`, installed-launcher
  checks, and the minimal default-journey integration test.

  **Accept:** `nix run` reaches an input-ready coding UI; an offline fixture run
  completes a prompt plus tool call; missing credentials produce actionable UI;
  successful live-provider use needs only supported credentials; every embedded
  source copied to disk behaves identically; `nix run .#pi-core` remains clean and
  its explicit file-backed journey passes. From this commit onward, default
  usability is a required check, not deferred release work.

## 4 — Complete the replaceable product vertically

Grow public modules only alongside their file-backed and shipped consumers. Keep
`nix run` green throughout. After 3.6, work may proceed by disjoint mechanism and
package paths, but shared binding indexes/default manifests remain serial.

- [ ] **4.1 — Close the public capability surface on demonstrated consumers**
  (**serial**; depends on 3.6).

  Add the remaining compact modules needed by configuration, records, richer UI,
  provider declarations, lifecycle/reload, and extension composition. Exercise
  every addition immediately from an ordinary file-backed package. Expose package
  modules, roots/declarations, display structures, async effects, provider/auth,
  and record operations without Pi/Node compatibility naming or mutable host
  access.

  **Accept:** file-backed applications can implement the complete agent loop,
  multi-component screen, all effect families, provider/auth operations, and
  arbitrary record persistence; every dispatch is bounded; generated concise API
  docs cover the actual demonstrated surface; embedded sources have no extra API.

After 4.1, `/orchestrate` may run **Wave P1** for the disjoint package trees.

- [ ] **4.2 — Config/resource package** (**Wave P1**, path owner:
  `crates/pi-rs-builtins/config/**`; depends on 4.1).

  Implement `config.lua` declarations, package/module selection, themes, keymaps,
  providers/models, tools, resource paths, and root selection. Load XDG first and
  legacy config only as fallback; project configuration has explicit trust policy
  and reload publishes atomically.

  **Accept:** precedence/trust/rollback/idempotence matrices pass; all effective
  configuration is inspectable; replacing the file-backed config changes policy;
  Rust contains no product default.

- [ ] **4.3 — Configurable session package** (**Wave P1**, path owner:
  `crates/pi-rs-builtins/session/**`; depends on 4.1).

  Implement optional persistent conversation policy over the public record store:
  record schema, reconstruction, names, branch/tree meaning, selection,
  compaction records, retention, and legacy interpretation. Session actions are
  queued and every write targets XDG.

  **Accept:** suppressing the package leaves the useful ephemeral app from 3.6;
  a small file-backed replacement persists a different schema; branch, compact,
  resume, corruption, cancellation, stale-handle, and legacy-read/XDG-write paths
  are covered.

- [ ] **4.4 — Integrate configuration/session and close replacement composition**
  (**serial after Wave P1**).

  Extend the declarative default manifest once, then independently suppress and
  replace application, agent, frontend, and session roots. Compose representative
  event/render middleware and config declarations. Prove deterministic conflicts,
  module versions, lifecycle cleanup, reload rollback, watchdog isolation, and
  copied-to-disk reproduction across the expanded graph.

  **Accept:** `nix run` remains input-ready with and without persistent sessions;
  each package/root is suppressible or replaceable; two extensions compose without
  privileged ordering; zero-builtin/file-backed checks remain green.

## 5 — Close the Pi-feeling interactive experience

After 4.4, `/orchestrate` may run **Wave P2** by separate Lua module trees and
fixture paths. One worker owns the frontend root integration points per wave;
other workers contribute modules through interfaces already merged.

- [ ] **5.1 — Transcript and streaming presentation** (**Wave P2**; depends on
  4.4).

  Complete user, assistant, thinking, tool, warning, error, retry, compaction,
  and custom rows with Pi's defining spacing/color/wrapping behavior. Streaming
  updates retain stable component identity and bounded invalidation.

  **Accept:** canonical transcript/tool/stream grids from 0.2 match; long
  transcripts remain within render and memory budgets; renderers are replaceable;
  the simpler 3.6 journey remains green.

- [ ] **5.2 — Editor, completion, and keymaps** (**Wave P2**; depends on 4.4).

  Complete Lua editor policy over terminal/text primitives: multiline edits,
  undo, history, paste collapse, file/path completion, command completion,
  external editor, and configurable keymaps.

  **Accept:** canonical input traces match; Unicode and large-paste cases pass;
  a file-backed editor/keymap replacement uses no private API.

- [ ] **5.3 — Dialogs, selectors, status, and chrome** (**Wave P2**; depends on
  4.4).

  Implement model/session selectors, generic select/confirm/input/editor dialogs,
  notifications, working indicator, header, footer, status, widgets, and overlays
  as Lua modules and public slots.

  **Accept:** canonical selector/dialog/footer grids and input traces match;
  every slot composes or replaces from a file-backed extension.

- [ ] **5.4 — Queueing, cancellation, and session UX integration** (**serial
  after 5.1–5.3**).

  Wire frontend actions to agent and optional session roots for steering,
  follow-ups, abort/restore, resume/new/fork/tree/compact, model changes, and
  graceful shutdown. Keep cross-root communication snapshot/action based.

  **Accept:** complete canonical interaction journeys pass without hidden mutable
  coupling; replacing or removing the session root requires no frontend fork;
  default and bare/file-backed Nix launch checks remain green.

## 6 — Complete provider and authentication parity

Provider/auth parity is intentionally exhaustive within the pinned supported
catalog. It may use Pi as a subsystem oracle. `/orchestrate` may split **Wave A**
by protocol/auth family only when implementation and fixture paths are disjoint.

- [ ] **6.1 — Protocol and model-catalog closure** (**Wave A**; depends on 5.4).

  Verify every advertised model dispatches to an implemented protocol family and
  every family has deterministic request/stream/error/cancellation replays.
  Preserve data-driven providers; do not clone transports per brand.

  **Accept:** catalog diff has no unexplained provider/model/API gaps; Anthropic,
  OpenAI Completions/Responses/Codex, Google/Vertex, Mistral, Bedrock, and other
  pinned advertised protocols pass focused parity fixtures.

- [ ] **6.2 — API-key and credential closure** (**Wave A**; depends on 5.4).

  Complete environment, config, command-backed, and stored credential resolution
  with deterministic precedence, redaction, refresh, and XDG/legacy behavior.

  **Accept:** every catalog provider has a tested auth path; no secret appears in
  logs/snapshots; legacy credentials are never modified.

- [ ] **6.3 — Subscription OAuth closure** (**Wave A**; depends on 5.4).

  Complete Anthropic, GitHub Copilot, and OpenAI/Codex subscription login,
  callback/device flows, refresh, logout, expiry, cancellation, and headless
  outcomes through generic auth mechanisms and Lua UI policy.

  **Accept:** deterministic flow fixtures and focused live-manual instructions
  cover every subscription provider; frontend login/logout is replaceable Lua.

- [ ] **6.4 — Provider configuration and selection UX** (**serial after Wave A**).

  Expose provider/model declarations and selection entirely through Lua config
  and product packages while retaining the full mechanism catalog.

  **Accept:** custom endpoints/models, model switching, thinking capability,
  missing-auth diagnostics, and reload all work without Rust product defaults.

## 7 — Performance, ablation, and release closure

- [ ] **7.1 — Meet measured performance budgets** (**serial measurement,
  path-owned optimization waves allowed**; depends on 6.4).

  Run the 0.2 release harness, profile failures, and optimize only measured hot
  paths. Batch snapshot/action conversion, retain display structures, bound
  history views, and remove unnecessary dependencies/features.

  **Accept:** startup, RSS, input-to-frame, sustained render, dispatch/effect,
  binary-size, and leak budgets in `DESIGN.md` pass through Nix on the reference
  environment. Results compare against the recorded baseline and explain
  variance.

- [ ] **7.2 — Final public-surface and ablation proof** (**serial**; depends on
  7.1).

  Delete every builtin package and run the bare/file-backed exercisers. Suppress
  and replace each shipped root/package and representative composable declaration.
  Audit Rust for product names, hardcoded policy, privileged embedded branches,
  mutable Lua host access, and duplicate declaration paths.

  **Accept:** zero-pack, per-package suppression, whole-root replacement,
  file-backed reproduction, stale-handle, watchdog, cancellation, and cleanup
  checks pass; no private capability remains.

- [ ] **7.3 — Release `pi`** (**serial**; depends on 7.2).

  Collapse migration notes and temporary manifests, generate concise Lua API and
  configuration documentation, verify XDG/legacy behavior, and build the release
  artifact through the flake.

  **Accept:** `nix flake check`, release `nix build`, and `nix run` pass from a
  clean checkout; the repository contains no stale faithful-port promise;
  `pi-rust-rewrite` is referenced only as historical provenance; tag the first
  pi-rs baseline.

## Permanent acceptance matrix

The final repository keeps only the smallest suites that independently protect:

1. Rust mechanism invariants and resource cleanup;
2. public file-backed Lua capability and source neutrality;
3. canonical Pi-feeling terminal grids/input journeys;
4. full provider protocol/model/auth subsystem parity;
5. XDG writes plus read-only legacy fallback;
6. optional/replaceable Lua session policy over the generic store;
7. zero-pack/per-package/root ablation;
8. measured release performance budgets.

Anything not serving one of these contracts is temporary scaffolding and is
removed when its milestone closes.
