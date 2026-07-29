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

- [x] **3.2 — Prove a file-backed coding walking skeleton** (**serial**; depends
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

  **Landed slice 2 (effect/model/cancellation through the loop):** the
  walking-skeleton example extends to bounded process effect execution
  (`effects.v1.process.run` renders `echo` stdout), missing-model
  diagnosis (`models.v1.find` returns `nil` for an unknown provider/model
  pair, and the Lua root renders a useful diagnostic frame), and
  cancellation (`effects.cancellation.new` + `AbortSignal:abort()`
  interrupting an in-flight `timer.sleep`, returning to an input-ready
  frame). The PTY acceptance test drives all three keys (`r`, `m`, `t`)
  through the public Lua surface over a real pseudo-terminal and verifies
  ANSI output for each before the shutdown exit. Lua 5.4 numeric literals
  avoid `_` separators (loader rejects them).

  **Landed slice 3 (fixture provider streaming through the loop):** the
  walking-skeleton example streams a deterministic fixture provider through
  the public `models.v1.stream` binding and renders every text delta as an
  incremental retained-display frame before the final result frame. The PTY
  acceptance test serves a canned OpenAI-completions SSE stream from an
  ordinary local HTTP fixture on `127.0.0.1:0`; the Lua package discovers
  the port by reading `fixture_port.txt` through the public
  `effects.v1.fs.read` effect, builds the model/context as ordinary Lua
  tables (`api = "openai-completions"`, `baseUrl` pointing at the fixture),
  and streams with only an `apiKey` option — no private channel, no
  synthetic embedded identity, no fixture protocol family in Rust. The test
  drives key `s` and verifies the three incremental delta frames (minimal
  cell diffs) plus the final `stream done: Hello, fixture world` frame over
  a real pseudo-terminal, then shuts down cleanly. Focused PTY test,
  `cargo fmt --check`, `cargo test --workspace` (59 suites, 0 failures), and
  `nix flake check` pass.

  **Landed slice 4 (agent/frontend root coordination):** the walking skeleton
  splits into three independently replaceable roots. `application.lua` owns
  only routing: startup frames come from the frontend root, raw bytes are
  decoded by the frontend root, and each key becomes one turn through the
  agent root, which renders through the frontend root. Every cross-root call
  uses the new public `roots.v1.dispatch(kind, event[, context])` seam: the
  nested root runs with its own bounded transaction and returns its settled
  batch to Lua as ordinary data; nothing publishes implicitly — the caller
  explicitly republishes chosen actions into its own batch. Nested execution
  shares the caller's runtime and watchdog budget, is depth-capped (8), and
  rejects direct recursion into a root kind already on the nest stack. The
  kernel transaction is now a stack so a nested dispatch restores the
  caller's transaction on success or error. Four focused invariants prove
  batch return/caller preservation, recursion rejection, nested-error
  non-publication, and priority-based root replacement through nested
  dispatch. The PTY acceptance test now loads `frontend.lua`, `agent.lua`,
  and `application.lua` in dependency order and drives the same key journey
  (`h`, `r`, `m`, `t`, `s`, `q`) through the coordinated roots over a real
  pseudo-terminal. Focused host tests (17), PTY test, `cargo fmt --check`,
  `cargo test --workspace`, and `nix flake check` pass.

  **Landed slice 5 (middleware composition and isolation invariants):** a new
  generic `crates/pi-rs-host/src/middleware.rs` mechanism adds
  `pi.roots.v1.middleware.register`, the sole declaration path for bounded
  stages around a root kind. An `event` stage runs before the resolved root
  and may replace the event, replace the queued actions, or `stop` the chain
  (the queued actions then become the batch); a `render` stage transforms the
  settled action list after the root succeeds, and a failing transform rolls
  the whole dispatch back so nothing publishes. Rust owns ordering (ascending
  `order`, then registration sequence), the per-stage watchdog, the 64-stage
  cap, and short-circuit semantics; every payload's meaning stays Lua policy.
  Stages are scope-owned like roots, so disposal and failed loads remove them,
  and identical `kind/phase/id` from a different source conflicts
  deterministically. Nested `roots.v1.dispatch` composes the same pipelines.
  `examples/walking-skeleton/middleware.lua` is an ordinary file-backed package
  that composes one event stage (lowercases the agent turn key, so typed `R`
  runs the effect demo) and one render stage (appends a `[mw]` marker action to
  any application batch that presented a frame) around roots it does not own;
  the PTY acceptance test asserts both from outside the process.
  `crates/pi-rs-host/tests/middleware_composition.rs` adds eight invariants:
  ordered event transform plus short-circuit, render transform, failing-render
  rollback, registration conflict plus load rollback, disposal cleanup,
  per-stage watchdog isolation, deterministic module version conflicts with
  exact-version resolution, and nested-dispatch composition.
  `cargo fmt --check`, `cargo test --workspace`, and `nix flake check` (which
  runs the workspace tests, clippy, the raw no-package guidance check, and the
  file-backed application check) pass.

  **Closed 2026-07-29:** every acceptance criterion is landed and integrated.
  The PTY harness drives the file-backed skeleton over a real pseudo-terminal:
  startup frame, typed echo, bounded process effect, missing-model diagnosis,
  cancellation of in-flight work, fixture-provider streaming with incremental
  frames, and clean shutdown with all scopes disposed. Application, agent, and
  frontend roots are independently replaceable, one event and one render
  middleware compose over them, and priority/conflict rules, module versions,
  watchdog isolation, rollback, and scope cleanup are proven on this surface.
  The falsifier did not fire: the loop is composed entirely from the public
  snapshot/action/effect contracts with no privileged escape.

  **Landed slice (loop mechanism):** the generic product loop composes the
  proven mechanisms end to end: terminal bytes → bounded input batches → root
  dispatch → action settlement → ANSI frames → shutdown, repeated until a
  shutdown action or stdin EOF. `crates/pi-rs-app/src/interactive.rs` owns the
  loop; `launcher.rs` splits one-shot (non-TTY or startup shutdown) from
  interactive (TTY, no startup shutdown) paths. The loop interprets two
  mechanism action kinds (`ansi` → present, `shutdown` → exit); all other
  action kinds remain Lua policy. `examples/walking-skeleton/application.lua`
  proves the loop from an ordinary file-backed package: startup renders an
  input-ready frame, typed input echoes through the retained display, and 'q'
  emits shutdown. The PTY acceptance test
  (`crates/pi-rs-app/tests/interactive_loop.rs`) spawns `pi` behind a real
  pseudo-terminal, verifies the startup frame, typed echo, and clean shutdown
  exit. Existing one-shot JSON mode is preserved for non-TTY use and startup-
  shutdown batches.

  **Remainder:** none. Root replacement and agent/frontend coordination landed
  in slice 4; process effects, missing-model diagnosis, and cancellation in
  slice 2; fixture streaming in slice 3; middleware composition, module version
  conflicts, watchdog isolation, and rollback in slice 5.

After 3.2, `/orchestrate` may run **Wave U**. Workers own disjoint package trees;
none may edit the default manifest, root binding indexes, or `flake.nix`.

- [x] **3.3 — Minimal ephemeral agent package** (**Wave U**, path owner:
  `crates/pi-rs-builtins/agent/**`; depends on 3.2).

  Implement the Lua agent reducer needed for a useful coding turn: prompt and
  provider stream consumption, sequential/parallel tool settlement, cancellation,
  bounded retry, and steering/follow-up queues. It has no persistence dependency
  and uses only the public modules proven by 3.2.

  **Accept:** deterministic fixtures cover text, tool use, cancellation, retry,
  malformed provider events, steering, and follow-up; replacing the agent root
  changes transition policy without a frontend fork.

  **Landed:** `crates/pi-rs-builtins/agent/{queue,tools,turn,init}.lua` is an
  ordinary file-backed package graph over the public spine only
  (`pi.roots.v1`, `pi.models.v1`, `pi.effects.v1`, `pi.kernel.v1.module`) with
  no persistence dependency. `init.lua` registers the `agent` root
  `pi.builtins.agent`; `pi.agent.turn@1` owns prompt/stream consumption,
  incremental `agent_text_delta` rendering, bounded retry (`max_retries = 2`,
  non-retryable missing model), tool settlement, interrupt cancellation,
  steering drained into the next request of the active turn, and follow-up
  turns bounded by `max_follow_ups`. `pi.agent.tools@1` is the one tool
  declaration path (`serialize` marks non-interleaving tools);
  `pi.agent.queue@1` supplies the bounded FIFOs.
  `crates/pi-rs-builtins/tests/agent_package.rs` drives 11 deterministic
  scenarios through a registered fixture api — text turn, parallel/serial tool
  groups, failing tool, missing model, transport retry recovery, retry-bound
  exhaustion, queued interrupt plus resumed turn, malformed provider events
  (undeclared tool call, empty delta, tool-use stop with no calls), steering +
  follow-up queues, and a higher-priority replacement agent root that changes
  transition policy with no frontend involved. `docs/lua-agent-package.md`
  records the event/action vocabulary for the 3.4/3.5 packages.
  `cargo fmt --check`, `cargo test --workspace` (63 suites, 0 failures), and
  `nix flake check` (workspace tests, clippy, raw no-package guidance,
  file-backed application) pass.

  **Decisions for later items:** (a) tool declarations live in the Lua module
  `pi.agent.tools@1` rather than a Rust declaration registry, because Wave U
  may not touch root binding indexes; 4.1 may promote it to a public
  declaration seam. (b) A "parallel" group is a bounded ordered settlement
  group, not concurrent execution: the public effect surface exposes no async
  handle yet, so concurrency needs a new mechanism seam before the wording can
  strengthen. (c) `crates/pi-rs-builtins` currently carries only
  `package_root()` plus this package tree; the distribution manifest, package
  indexes, and embedding remain 3.6's.

- [x] **3.4 — Minimal application/frontend package** (**Wave U**, path owner:
  `crates/pi-rs-builtins/frontend/**`; depends on 3.2).

  Implement the Lua application/frontend roots, retained component tree, basic
  multiline input, transcript rows, focus/input routing, resize, invalidation,
  streaming updates, missing-auth/model guidance, and graceful shutdown. Keep
  editor/transcript/chrome modules separate even where behavior is initially
  sparse; Rust receives display/effect actions only.

  **Accept:** fixture journeys reach an input-ready frame, submit a prompt, show
  incremental assistant/tool output, cancel, and exit; file-backed roots can
  replace the application or frontend and wrap the render middleware.

  **Landed:** `crates/pi-rs-builtins/frontend/{keys,editor,transcript,chrome,
  view,init,application}.lua` is an ordinary file-backed package graph over the
  public spine only (`pi.roots.v1`, `pi.terminal.v1`, `pi.kernel.v1.module`)
  with no persistence dependency. `init.lua` registers the frontend root
  `pi.builtins.frontend` (sole owner of the retained display and bounded input
  buffer) and `application.lua` registers the coordinator root
  `pi.builtins.application`, which republishes only `ansi`/`shutdown` to Rust
  and keeps every product action inside the Lua roots. Editor, transcript, and
  chrome stay separate modules; `pi.frontend.view@1` builds one display batch
  with stable node identities (header/transcript/guidance/editor/footer), so an
  unchanged region is retained and only changed cells paint. Key routing goes
  through the focused component, resize repaints via `reset_presentation`, and
  each transcript change renders, so assistant text and tool rows appear
  incrementally. `crates/pi-rs-builtins/tests/frontend_package.rs` drives 12
  deterministic journeys through the application root with a registered fixture
  api — input-ready startup frame, typed prompt with ≥4 incremental frames,
  tool start/result rows, interrupt then cancelled turn, ctrl+d shutdown,
  resize repaint at a new size, missing-model guidance, rejected-credential
  guidance with bounded retry, multiline editing (alt+enter, backspace),
  file-backed frontend replacement, file-backed application replacement driving
  the shipped frontend, and file-backed render middleware wrapping the shipped
  frame. `docs/lua-frontend-package.md` records the module, event, action, and
  intent vocabulary. `cargo fmt --check`, `cargo test --workspace` (64 suites,
  0 failures), and `nix flake check` (workspace tests, clippy, raw no-package
  guidance, file-backed application, model-catalog update) pass.

  **Decisions for later items:** (a) terminal size is not a public mechanism
  yet, so the frontend starts at 80×24 and adopts real dimensions from a
  `resize` event; the launcher gains that event when the size/resize mechanism
  lands. (b) Transcript rows are clipped, not wrapped, and there is no
  scrollback command surface — richer presentation is 5.1–5.3. (c) The
  application root reads its model from a `configure`/`startup` event payload;
  configuration files and provider selection UX remain 4.2/6.4.

- [x] **3.5 — Minimal core-tool package** (**Wave U**, path owner:
  `crates/pi-rs-builtins/tools/**`; depends on 3.2).

  Ship `read`, `write`, `edit`, and `bash` as Lua declarations over public
  filesystem/process/diff effects. Tool execution, truncation, cancellation, and
  file-mutation serialization are policy in this package. Do not restore inherited
  utility mega-chunks or add a privileged builtin executor.

  **Accept:** each tool is independently suppressible/replaceable from disk;
  concurrent mutations are safe; path errors, bounded output, process-tree
  cancellation, and representative render data are covered.

  **Landed:** `crates/pi-rs-builtins/tools/{paths,render,locks,read,write,edit,
  bash,init}.lua` is an ordinary file-backed package graph over the public
  spine only (`pi.effects.v1`, `pi.roots.v1`, `pi.kernel.v1.module`) plus the
  one tool declaration path `pi.agent.tools@1`. No privileged executor: each
  tool is a module exposing `execute/declare/unregister`, and `init.lua` only
  declares them through `pi.tools.suite@1`
  (`suppress`/`tools`/`shared` options). `pi.tools.paths@1` owns lexical path
  policy (workspace root, `..` escape and absolute-path rejection),
  `pi.tools.render@1` owns line windows, byte-bounded clipping with an explicit
  truncation notice, and bounded line diffs, and `pi.tools.locks@1` owns
  file-mutation serialization (per-path `guard` for `write`/`edit`, one
  workspace slot for `bash`, plus a per-path revision used by `edit`'s
  `expected_revision` guard). `bash` runs its command as a `set -m` job in its
  own process group, learns the group id from a marker it strips from output,
  and kills that group on cancellation or timeout, so backgrounded grandchildren
  die with the command. `crates/pi-rs-builtins/tests/tools_package.rs` drives 15
  deterministic scenarios through a file-backed driver root — numbered reads,
  line windows plus truncation, four path-error shapes, create/update writes
  with diff render rows, oversize rejection, unique/ambiguous/missing/
  `replace_all` edits, the stale-revision guard, shell exit codes and stderr,
  bounded output, timeout kill, process-tree cancellation (a backgrounded
  grandchild never lands its marker file), declared serialization plus a busy
  path lock, and per-tool suppression/replacement from disk.
  `docs/lua-tools-package.md` records the modules, arguments, result data, and
  replacement recipe. `cargo fmt --check`, `cargo test --workspace` (65 suites,
  0 failures), and `nix flake check` (workspace tests, clippy, raw no-package
  guidance, file-backed application, model-catalog update) pass.

  **Decisions for later items:** (a) there is no public diff effect; diffs are
  Lua render policy in `pi.tools.render@1`, which needs no new mechanism. (b)
  Tool results carry a `details` render table, but the shipped agent forwards
  only `output`/`is_error`; promoting `details` to a public tool-result seam is
  4.1 work (Wave U may not touch `agent/**`). (c) Relative paths in
  `pi.effects.v1.fs` resolve against the host process directory, not the host
  `cwd` config, so tools take an explicit `root` option; 3.6/4.2 decide who
  supplies it by default. (d) Cancellation is observed at command output
  boundaries; a silent command is bounded only by its timeout until an async
  effect handle exists (same limit as 3.3's parallel settlement).

- [x] **3.6 — Assemble defaults and restore `nix run`** (**serial after Wave U**).

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

  **Landed:** `crates/pi-rs-builtins/default.json` is the one declarative
  distribution manifest — an ordinary version 1 launcher manifest whose 20
  package paths resolve from its own directory, so the repository copy, the
  Nix-store copy, and a user's copy are the same file. Nothing is embedded or
  concatenated: `pi-rs-builtins` still ships only Lua plus `package_root()`/
  `manifest_path()`. `crates/pi-rs-builtins/defaults/init.lua` is the only
  added policy, as two public application event middleware stages —
  `pi.builtins.defaults.model` injects the first catalog model of a declared
  candidate list (`anthropic/claude-sonnet-4-5`, `openai/gpt-5.1`,
  `openrouter/anthropic/claude-sonnet-4.5`) into a startup event that carries
  none, and `pi.builtins.defaults.tool-root` re-declares the shipped tool
  suite with `root = snapshot.context.root` on the first dispatch (this closes
  3.5's open question (c)). Both are replaceable by id, and neither reads a
  credential: `pi.models.v1.stream` resolves the provider's supported key
  itself. `flake.nix` gained `mkPiPackages`/`mkPiRs`: the default package
  copies the package trees plus the manifest to `share/pi/packages` and wraps
  `pi-core` with `--set-default PI_PACKAGE_MANIFEST`, so `--package`,
  `--manifest`, and an explicit env selection all still win; `pi-core` remains
  the unwrapped zero-builtin target.
  `crates/pi-rs-app/tests/default_distribution.rs` (6 tests) covers manifest ↔
  package-tree agreement, the installed-launcher startup batch (ansi-only, no
  shutdown, shipped application root as `source`), default model selection
  without configuration, byte-identical frames from a copied-to-disk
  distribution, the offline fixture journey (prompt → shipped `read` tool row
  → assistant follow-up → idle), and credential guidance.
  `docs/default-distribution.md` records the manifest, selection precedence,
  defaults, and the replacement recipe. New Nix check `default-distribution`
  runs the installed wrapper: `--help`, an input-ready frame carrying the
  default model, and an explicit `--package` override beating the manifest.
  `cargo fmt --check`, `cargo test --workspace`, and `nix flake check`
  (workspace tests, clippy, default distribution, raw no-package guidance,
  file-backed application, model-catalog update) pass.

  **Decisions for later items:** (a) the default model list is Lua constants
  in the defaults package; file-backed configuration and provider selection UX
  stay 4.2/6.4, which should replace this stage rather than add a Rust
  default. (b) Lua still has no public environment mechanism, so the
  distribution cannot key defaults off env vars; add it in 4.1 if 4.2 needs
  it. (c) The Nix check exercises the non-TTY startup batch (the same batch an
  interactive session presents first); the TTY input loop stays covered by
  `crates/pi-rs-app/tests/interactive_loop.rs`, not by Nix. (d) **Unrun live
  check:** a real provider round trip needs a supported credential and network
  and was not executed; only the offline fixture journey is evidence. (e)
  There are still no embedded package sources at all, so "embedded copied to
  disk behaves identically" is proven by copying the shipped files; if an
  embedded source is ever added, that test must compare both loads.

## 4 — Complete the replaceable product vertically

Grow public modules only alongside their file-backed and shipped consumers. Keep
`nix run` green throughout. After 3.6, work may proceed by disjoint mechanism and
package paths, but shared binding indexes/default manifests remain serial.

- [x] **4.1 — Close the public capability surface on demonstrated consumers**
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

  **Landed slice (records):** `pi.records.v1` is the sixth public module and the
  first consumer-demonstrated 4.1 addition, closing the "arbitrary record
  persistence" criterion. `crates/pi-rs-host/src/bindings/records.rs` exposes the
  2.3 record store as `create`/`open`/`list` plus store
  (`path`, `record_count`, `append`, `cursor`, `copy`, `close`, `closed`) and
  cursor (`next_sequence`, `next`) operations, all snake_case and versioned;
  destinations, names, limits, and schema stay entirely in Lua, so no resource
  path or session meaning entered Rust. Bounds: windows are capped by
  `default_limits` (1 MiB record, 256 records, 4 MiB per window), oversize
  records are rejected, and listings report locked/corrupt files as explicit
  diagnostics. `crates/pi-rs-host/src/kernel_api.rs` extracts
  `register_scoped_resource` so host mechanisms use the same registration and
  disposal path as `pi.kernel.v1.resource`: every open store is a scope resource
  whose disposer closes the file lock at package disposal instead of at Lua GC.
  Records observe the innermost dispatch cancellation (`current_cancellation`) or
  an explicit kernel token; operations are synchronous, so a cancelled token
  fails the call before blocking work. Evidence:
  `crates/pi-rs-host/tests/records_store.rs` (2 tests) drives the whole journey
  from an ordinary file-backed package — persist three arbitrary-schema records
  at a destination read from `snapshot.context`, atomic prefix copy, bounded
  cursor window, listing diagnostics for locked stores, limit and closed-store
  rejection, on-disk format bytes, an untouched read-only legacy resource, and
  scope-resource accounting where a second host may take the lock only after
  `dispose_package`. `crates/pi-rs-host/tests/public_surface.rs` now pins the
  six-module shape and keeps embedded/file provenance identical.
  `docs/lua-extension-api.md` documents the module. `cargo fmt --all -- --check`,
  `cargo test --workspace`, and `nix flake check` pass.

  **Landed slice (environment, paths, filesystem metadata):** the second
  consumer-demonstrated 4.1 addition closes 3.6 decision (b) and the effect
  families a configuration/resource policy needs. `pi.effects.v1` gained three
  members, all in `crates/pi-rs-host/src/bindings/effects.rs`: `env` is an
  immutable startup snapshot read by name (`get`, sorted `names`) with no bulk
  value dump and no write path; `path` is pure POSIX arithmetic (`join`,
  `normalize`, `dirname`, `basename`, `extname`, `is_absolute`, `resolve`,
  `relative`, `separator`); `fs` gained `exists`, `stat`, `list`,
  `make_directory`, and `remove_file`. Bounds: listings default to 1024 entries
  and are capped at 16384 (`default_max_entries`/`max_entries`); every operation
  keeps the existing effect-hub cancellation and scope ownership. Rust names no
  directory, precedence, or variable meaning: `HostConfig::environment`
  (`crates/pi-rs-host/src/lib.rs`, snapshotted once in
  `crates/pi-rs-host/src/bindings.rs`) only chooses which environment the VM
  sees, defaulting to the process environment at host start, which also keeps
  tests deterministic without mutating process state. Evidence:
  `crates/pi-rs-host/tests/configuration_paths.rs` (5 tests) drives the whole
  XDG/legacy matrix from an ordinary file-backed package whose resolution chain
  is Lua — explicit `XDG_CONFIG_HOME` wins, an unset variable falls back to the
  conventional `HOME/.config` location, legacy is used only as an untouched
  read-only fallback that never creates the preferred path, and an absent
  configuration leaves Lua constants and creates nothing — plus state-directory
  creation/write/list/stat/re-read/removal, listing-bound and missing-path
  failures, and the inherited process snapshot.
  `crates/pi-rs-host/tests/public_surface.rs` now also pins the exact
  `pi.effects.v1` family list. `docs/lua-extension-api.md` documents the three
  members and their bounds. `cargo fmt --all -- --check`, `cargo test
  --workspace`, and `nix flake check` (8 checks: workspace tests, clippy,
  default distribution, raw no-package guidance, file-backed application,
  model-catalog update) pass.

  **Landed slice (package composition and lifecycle):** `pi.packages.v1` is the
  seventh public module and the third consumer-demonstrated 4.1 addition. A
  package may now compose other packages:
  `crates/pi-rs-host/src/bindings/packages.rs` exposes `load{path=...}` /
  `load{name=..., source=...}`, `list()`, and a handle with `source`, `scope`,
  `dispose`, `disposed`. Rust names no location, order, generation, or reload
  policy: requests are explicit, `PackageSource::resolve` does the byte loading,
  so provenance stays the only difference between a host-loaded and a
  Lua-loaded package. Mechanism: `crate::vm::load_nested` /`dispose_nested`
  reuse the canonical scope creation, attribution, failure cleanup, and
  disposal order; `crate::vm::nested_function` runs nested chunks and disposers
  under the caller's watchdog hook, so composition consumes the one bounded
  dispatch budget and never starts a nested `block_on`
  (`kernel_api::dispose_callbacks_async`). `kernel_api::suspend_transaction`
  stacks the caller's publication queue for the duration, so a loaded chunk
  cannot append to the loading dispatch's batch. Every loaded package is
  registered as one disposable resource of its loader through
  `register_scoped_resource`, so disposing a composing package disposes what it
  composed, transitively. Bounds: 4 nested loads, 64 simultaneous Lua-loaded
  packages, 4 MiB per source, pruned records, refusal to dispose the running or
  still-loading package, and the existing duplicate-source conflict.
  Evidence: `crates/pi-rs-host/tests/package_lifecycle.rs` (3 tests) drives the
  journey from ordinary file-backed packages — a supervisor composes a child
  read from an env-supplied directory and serves its declaration; disposing the
  supervisor cascades into the child's own disposer and leaves no root; a swap
  to a failing package publishes the failure and keeps the previous generation
  selected, a swap to a good package selects the new generation and disposes
  the old, and re-composing a loaded source is refused; a six-level chain stops
  at the fourth nested load with the bound named in the diagnostic.
  `crates/pi-rs-host/tests/public_surface.rs` pins the seven-module shape and
  the exact `pi.packages.v1` member list. `docs/lua-extension-api.md` documents
  the module, its bounds, and that atomic replacement is Lua policy (load the
  new generation, then dispose the old). `cargo fmt --all -- --check`, `cargo
  test --workspace`, and `nix flake check` (8 checks) pass.

  **Landed slice (module lifecycle/reload):** the fourth consumer-demonstrated
  4.1 addition removes "redefinition needs package disposal".
  `crates/pi-rs-host/src/module_api.rs` adds `remove(name, version)` and
  `reset(name, version)` to `pi.kernel.v1.module`. `define` is unchanged and
  still refuses a duplicate identity, so there stays exactly one declaration
  path per kind: replacement is explicitly `remove` then `define` (the shape
  `pi.packages.v1` already uses for generation swaps), and re-running a factory
  without changing its declaration is `reset`. `remove` prunes the order index,
  so a redefined identity is listed once in its new position; `reset` drops only
  the cached value, so the same factory and dependency aliases run again. Both
  are scope-local through the shared `scope_for_current_entry` owner check: a
  sibling package gets an error naming the owning source and keeps read access,
  and a factory that is mid-resolution is refused rather than silently unwound.
  Rust invalidates nothing else and offers no module cleanup hook — dependent
  reload order is Lua policy, and a module that owns something disposable
  exposes an ordinary `pi.kernel.v1.resource` handle its owner disposes before
  reloading. Evidence: `crates/pi-rs-host/tests/module_lifecycle.rs` (2 tests)
  drives both journeys from ordinary file-backed packages — a running package
  caches one factory run, disposes its module's resource, resets to re-run the
  same factory, then swaps the implementation of a live identity with
  `remove` + `define` while `list()` still reports it once, and the package's
  scope ends with zero live resources; a sibling's `remove`/`reset` is refused
  by owner while `require` still works, a self-removing factory is refused as
  loading, and package disposal remains the other lifecycle path.
  `crates/pi-rs-host/tests/public_surface.rs` now also pins the exact
  `pi.kernel.v1` and `pi.kernel.v1.module` member lists.
  `docs/lua-extension-api.md` gains a module-lifecycle section.
  `cargo fmt --all -- --check`, `cargo test --workspace` (71 suites, 0
  failures), and `nix flake check` (8 checks: workspace tests, clippy, default
  distribution, raw no-package guidance, file-backed application, pi-core
  package, model-catalog update) pass.

  **Landed slice (provider inventory and model-row validation):** the fifth
  consumer-demonstrated 4.1 addition closes the *provider declaration* half of
  the provider/auth criterion. `pi.models.v1` gained `providers()`,
  `catalog(provider[, {offset=, limit=}])`, `apis()`, `validate(row)`, and its
  bound constants (`default_max_models` 64, `max_models` 512,
  `default_max_events` 256, `max_events` 1024);
  `crates/pi-rs-host/src/bindings/models.rs` owns the window bounds and
  `crates/pi-rs-host/src/ai.rs` owns the catalog/api/validation bridge. No new
  declaration path appeared: a provider declaration is
  `pi.kernel.v1.declare("provider", ...)` read back through
  `registered("provider")`, so Rust names no provider, endpoint, order, or
  selection rule — it only reports which reviewed catalog rows exist, which
  wire-protocol families can dispatch, and whether a package-authored row is
  wire-valid. `validate` stores nothing and a catalog row validates to itself,
  so a custom endpoint and a catalog row are the same kind of value.
  `pi_rs_ai::registry::ensure_builtin_api_providers` (was the private
  `ensure_builtins` in `crates/pi-rs-ai/src/registry/stream.rs`) is now public
  so inspecting the api families sees the same registry a stream dispatches
  through, without streaming first. Bounds: windows are `1..=512` rows and
  return the full row count as a second value, so paging never copies the
  969-row catalog. Evidence:
  `crates/pi-rs-host/tests/provider_declarations.rs` (2 tests) drives both
  journeys from ordinary file-backed packages — one package reads the
  inventory, declares a catalog-backed provider and a custom-endpoint provider
  through the generic declaration path, selects by its own declared order, and
  streams `Hello, declared provider` from a local fixture endpoint whose port
  arrives through the public environment/filesystem effects; the other pins
  every declaration-time refusal (zero/oversize window, unregistered api naming
  the supported families, empty `id`/`provider`/`baseUrl`, an incomplete row's
  missing-field diagnostic) and that an unknown provider is an empty window,
  not an error. `crates/pi-rs-host/tests/public_surface.rs` now also pins the
  exact `pi.models.v1` member list. `docs/lua-extension-api.md` documents the
  inventory, validation, and the declaration split. `cargo fmt --all --
  --check`, `cargo test --workspace` (71 suites, 0 failures), and `nix flake
  check` (8 checks: workspace tests, clippy, default distribution, raw
  no-package guidance, file-backed application, pi-core package, model-catalog
  update) pass.

  **Landed slice (credential storage and resolution):** `pi.auth.v1` is the
  eighth public module and the sixth consumer-demonstrated 4.1 addition,
  closing the *credential* half of the provider/auth criterion.
  `crates/pi-rs-host/src/bindings/auth.rs` exposes `providers()` (the
  subscription identities that can refresh a stored OAuth row) and
  `store{canonical=[, legacy=]}`, whose handle offers `snapshot`, `describe`,
  `set_api_key`, `set_oauth`, `remove`, and `resolve`. Both file locations are
  arguments, so Rust names no path, no product provider, and no precedence
  rule; the only refusals are ones Rust cannot implement (a relative path, a
  legacy fallback equal to the canonical file, a blank provider id). Secrets
  leave Rust through exactly one member: `snapshot` reports provenance
  (`canonical`/`legacy`/`absent`) and provider names, `describe` reports kind,
  expiry, and provider-defined extra-field names, and only `resolve` returns
  `{api_key, refreshed}`. Mechanism reused unchanged from
  `pi_rs_ai_auth::CredentialStore`: canonical-first selection with a read-only
  legacy fallback, inter-process locking, atomic owner-private replacement,
  stored-value expansion (`$NAME` from the process environment, `!command`
  through the shell with its own hard timeout and cache), and OAuth refresh
  written back under the same lock. Bounds: 64 KiB per stored or resolved
  secret (`max_secret_bytes`), 256 providers per snapshot (`max_providers`).
  Every mutating member is async and races the innermost dispatch cancellation
  (`current_cancellation`); the store owns no OS resource between calls, so
  there is nothing to register as a scope resource and the canonical write
  itself is synchronous and atomic. Evidence:
  `crates/pi-rs-host/tests/credential_store.rs` (2 tests) drives the whole
  journey from ordinary file-backed packages whose locations come from the
  public environment/path effects — legacy selected and resolved while
  canonical is absent, the first write promoting to canonical and migrating the
  legacy rows forward while leaving the legacy file byte-identical, a
  command-backed api-key expression expanded only at `resolve`, an OAuth row
  keeping its extra fields with expiry reported but no token exposed, removal,
  owner-private canonical mode bits, and the subscription inventory; the other
  pins absence-as-data and every refusal (relative path, identical paths, blank
  provider, oversize secret, unknown OAuth provider, incomplete OAuth row).
  `crates/pi-rs-host/tests/public_surface.rs` now pins the eight-module shape
  and the exact `pi.auth.v1` member list. `docs/lua-extension-api.md` gains a
  credentials section recording the surface, the bounds, and the deliberate
  divergence that stored-value expansion reads the live process environment
  while `pi.effects.v1.env` is the immutable startup snapshot.
  `cargo fmt --all -- --check`, `cargo test --workspace` (72 suites, 0
  failures), and `nix flake check` (8 checks: workspace tests, clippy, default
  distribution, raw no-package guidance, file-backed application, pi-core
  package, model-catalog update) pass.

  **Landed slice (subscription login):** the seventh consumer-demonstrated 4.1
  addition closes the *login* half of the provider/auth criterion, so
  `pi.auth.v1` now covers the whole credential lifecycle. `login(provider,
  callbacks[, options])` in `crates/pi-rs-host/src/bindings/auth/login.rs` runs
  one subscription OAuth flow and returns the credential row — the same shape
  `set_oauth` accepts — instead of storing it, so which store receives the row
  stays the same Lua decision as every other credential location. Rust names no
  login step: it runs the wire flow only (PKCE, the loopback callback server,
  authorization-code exchange, RFC 8628 polling), and every user-visible step is
  an ordinary Lua function — `on_auth`, `on_device_code`, `on_prompt`,
  `on_select` (required), plus optional `on_progress`, `on_manual_code_input`
  (its presence is what enables the manual-entry race), and `model_ids`. No new
  declaration path appeared: the flows are the existing registry rows already
  reported by `providers()`. Mechanism: a `Send + Sync` bridge holding no Lua
  value turns each callback into a queued request with a `oneshot` reply, which
  is what lets a non-`Send` Lua VM drive a `Send` provider flow; a second future
  serves that queue in arrival order with at most one Lua call in flight, and is
  polled concurrently with the flow, so a pending manual-code prompt does not
  stop the callback server from settling. Reply channels carry successes only —
  a raising Lua callback ends the login through the driver. Bounds: `timeout_ms`
  (`default_login_timeout_ms` 900000, `max_login_timeout_ms` 3600000), 128
  `model_ids` entries, and the existing 64 KiB secret bound applied to pasted
  codes and prompt answers. Every step races the innermost dispatch cancellation
  (`current_cancellation`), and the cancel branch carries `error::CANCEL_MARKER`
  so a cancelled login reports as `HostError::Cancelled` like any other
  cancelled dispatch; the callback server is owned by the flow future, so
  dropping the login releases its port. Evidence:
  `crates/pi-rs-host/tests/subscription_login.rs` (4 tests) drives every journey
  from ordinary file-backed packages against fixture provider rows pointed at a
  local HTTP socket, so the whole thing runs offline — a browser login where Lua
  picks the method from the flow's options, shows the authorization URL, pastes
  the code back, then stores and resolves the returned row (extras such as
  `accountId` preserved); a headless device-code login where Lua answers the
  enterprise prompt, renders the code with its polling parameters, and supplies
  the catalog rows enabled afterwards; every pre-flight refusal (unknown
  provider, missing required callback, zero/oversize timeout, oversize
  `model_ids`); and a login parked on an endpoint that never answers, ended by
  `dispose_package` and reported as a cancelled dispatch.
  `crates/pi-rs-host/tests/public_surface.rs` now pins the nine-member
  `pi.auth.v1` list. `docs/lua-extension-api.md` gains a subscription-login
  section recording the callbacks, the bounds, and the callback-ordering rule.
  `cargo fmt --all -- --check`, `cargo test --workspace` (74 suites, 0
  failures), and `nix flake check` (8 checks: workspace tests, clippy, default
  distribution, raw no-package guidance, file-backed application, pi-core
  package, model-catalog update) pass. **Unrun live check:** a real subscription
  login against Anthropic, GitHub, or OpenAI needs an account and network and
  was not executed; only the fixture-endpoint journeys are evidence.

  **Landed slice (text measurement for Lua layout):** the eighth
  consumer-demonstrated 4.1 addition adds the first display structure beyond the
  retained tree — the cell arithmetic a package needs to decide what to put in
  that tree. `pi.terminal.v1.text` offers `width`, `measure`, `wrap`,
  `truncate`, and `graphemes` plus their bounds
  (`max_bytes` 1 MiB, `default_max_graphemes` 1024 / `max_graphemes` 16384,
  `default_max_rows` 1024 / `max_rows` 16384). Until now Lua could submit a
  retained tree but could not compute a single Unicode cell width, so a package
  had to guess with byte or codepoint counts — the shipped
  `crates/pi-rs-builtins/frontend/view.lua` still measures its prompt with `#`,
  which is correct only while that prompt stays ASCII. Mechanism: the paint
  loop in `crates/pi-rs-tui/src/display.rs` is refactored into one shared
  `walk_text` traversal that both `paint_text` and the new public
  `text_width`/`measure_text`/`wrap_text`/`truncate_text`/`text_graphemes` use,
  so measurement cannot drift from rasterization: a node sized from `measure`
  paints exactly `measure(...).cells`, and `wrap` returns the rows the frame
  actually holds. The primitives reject exactly the control data
  `RetainedDisplay::submit` rejects, so text that measures is text that submits;
  single-line members additionally refuse newline and tab, which change layout
  rather than width. Rust names no appearance: grapheme wrapping breaks at the
  last cluster that fits and never moves a word, the ellipsis and the budget are
  arguments, row budgets clip and report overflow instead of allocating, and
  where the caret sits is Lua summing cluster widths. Every call is synchronous
  and bounded by input bytes and window size, so no dispatch, allocation, or
  per-cell crossing grows with terminal size.
  `crates/pi-rs-host/src/tui_api/text.rs` owns the Lua-facing bounds and
  `crates/pi-rs-host/src/bindings/terminal.rs` installs the table. Evidence:
  `crates/pi-rs-host/tests/text_layout.rs` (2 tests) drives the whole journey
  from ordinary file-backed packages — one package wraps a mixed
  wide/combining/tab/newline paragraph, truncates a footer with its own
  ellipsis, places a caret from cluster widths, and predicts the submitted
  frame's `painted_cells` (24) from measurement alone before submitting; the
  other pins every refusal (tab/newline in single-line width, escape data in
  both `width` and `submit`, zero width, zero tab width, missing width, unknown
  wrap mode, out-of-range windows, oversize input) and the bounded behaviours
  (row-budget clipping with an overflow flag, offset windows with the total
  count, an ellipsis wider than the budget, and text that already fits).
  `crates/pi-rs-tui/src/display.rs` gains two unit tests proving the
  measure-equals-paint invariant against a real rasterized frame and the
  primitive-level refusals. `crates/pi-rs-host/tests/public_surface.rs` now pins
  the exact `pi.terminal.v1` and `pi.terminal.v1.text` member lists.
  `docs/lua-extension-api.md` gains a text-measurement section.
  `cargo fmt --all -- --check`, `cargo test --workspace` (74 test targets, 313
  tests, 0 failures), and `nix flake check` (8 checks: workspace tests, clippy,
  default distribution, raw no-package guidance, file-backed application,
  pi-core package, model-catalog update) pass.

  **Landed slice (hyperlink styling):** the ninth consumer-demonstrated 4.1
  addition adds the first display content a run can carry beyond glyphs. A text
  run may now set `link="<target>"` beside `text` and `style`
  (`TextRun.link` in `crates/pi-rs-tui/src/display.rs`, parsed in
  `crates/pi-rs-host/src/tui_api/runtime.rs`); every cell that run paints holds
  the target and the differential presenter wraps exactly those cells in one
  OSC 8 sequence, closing it on the first cell that does not carry the same
  target. Mechanism, not appearance: `Cell` gained a `link` field, so a
  target-only change is an ordinary cell change and repaints just that span,
  and `write_cells` tracks hyperlink state separately from SGR state because a
  style reset does not end a link. The shared `walk_text` traversal now carries
  a `RunAttributes { style, link }` payload instead of a bare `CellStyle`, so
  measurement and rasterization still cannot diverge and a link costs one `Arc`
  per run rather than one per cell. Rust names no appearance: no underline, no
  color, no OSC 8 `id` grouping, and no opinion about what is linkable. Bounds:
  `max_link_bytes` (default 65536, `display({max_link_bytes=...})`) caps total
  target bytes per batch; an empty target (the OSC 8 close sequence) and any
  control character (which would terminate the sequence early) are refused
  before anything is retained. `DISPLAY_SCHEMA_VERSION` is now `2` because the
  batch schema gained a field; every package already reads
  `pi.terminal.v1.display_schema_version`, so the only pinned consumer updated
  was `crates/pi-rs-host/tests/tui_terminal.rs`. Evidence:
  `crates/pi-rs-host/tests/display_links.rs` (2 tests) drives the journey from
  ordinary file-backed packages — a transcript row where Lua owns the label,
  the target, and the underline styling asserts that the sequence opens before
  the label and closes before the unlinked tail, that the row still paints
  twelve cells because a target is not glyphs, that retargeting the same text
  changes exactly four cells, and that resubmitting emits nothing; the other
  pins every refusal (empty, BEL, ESC, newline targets, and an oversize batch
  naming the byte count and limit) with the display revision still 0.
  `crates/pi-rs-tui/src/display.rs` gains two unit tests proving the OSC 8
  placement against a real rasterized frame, that all cells of a run share one
  `Arc`, and the validation refusals. `docs/lua-extension-api.md` gains a
  hyperlink section and `docs/lua-coding-spine.md` records schema version 2.
  `cargo fmt --all -- --check`, `cargo test --workspace` (75 test targets, 317
  tests, 0 failures), and `nix flake check` (8 checks: workspace tests, clippy,
  default distribution, raw no-package guidance, file-backed application,
  pi-core package, model-catalog update) pass. **Deliberate omission:** the OSC
  8 `id=` parameter, which joins non-contiguous cells into one hover/underline
  region, is not exposed; a wrapped link therefore reads as one region per row.
  Add it only with a consumer that needs it.

  **Landed slice (inline terminal images):** the tenth consumer-demonstrated 4.1
  addition closes "display node content beyond group/text". A node's content may
  now be `{kind="image", data="<base64>", protocol="kitty"|"iterm2"}`
  (`DisplayNodeContent::Image` in `crates/pi-rs-tui/src/display.rs`, parsed in
  `crates/pi-rs-host/src/tui_api/runtime.rs`), and the node's own `rect` is the
  placement. This is the *second* out-of-band mechanism and deliberately not the
  hyperlink one: a link is per-cell state the differential cell diff already
  carries, while an image is one escape addressed at a cursor position covering a
  rectangle, so it cannot be reconstructed from cell diffs. Images therefore
  never enter the cell grid at all — `Frame` gained `images: Vec<FrameImage>`,
  and `present_images` compares whole placements after the cell pass. Identity is
  the mechanism that makes removal possible: `ImageIdentities` assigns each image
  node a terminal-side id, stable for the life of one `RetainedDisplay` and never
  reused, so replacing a payload deletes that id before transmitting the
  replacement and dropping the node deletes it outright — blanking cells does not
  remove a graphic. Because repainting cells *does* draw text over one, any
  placement whose rows the cell pass rewrote is re-emitted; an unchanged image
  over untouched rows emits nothing. Rust names no terminal, scaling, aspect
  ratio, z-order, or overlap rule: protocol choice is Lua policy computed from
  `pi.effects.v1.env`, and an image whose rectangle is not fully inside its clip
  is skipped rather than partially drawn. Kitty placements pass `C=1` so the
  cursor restore stays authoritative. Bounds: `max_images` (default 16) and
  `max_image_bytes` (default 4 MiB) per batch, and a payload outside the standard
  base64 alphabet is refused before anything is retained, because it is spliced
  verbatim into an escape sequence. `SubmitResult` gained `placed_images`, kept
  separate from `painted_cells` because an image paints no cell.
  `DISPLAY_SCHEMA_VERSION` is now `3`; every package reads
  `pi.terminal.v1.display_schema_version`, so the only pinned consumers updated
  were `crates/pi-rs-host/tests/{tui_terminal,display_links}.rs`. Evidence:
  `crates/pi-rs-host/tests/display_images.rs` (2 tests) drives the journey from
  ordinary file-backed packages — a package that picks its protocol from
  `TERM_PROGRAM` places a 6x2 image at row 1 column 2, asserts the addressed
  placement and `C=1,c=6,r=2,i=1`, that the text row above still counts 5 painted
  cells while the image counts 1 placed image, that resubmitting emits nothing,
  that a new payload deletes id 1 before transmitting, and that dropping the node
  deletes without re-transmitting; the other pins every refusal (empty, escape-
  bearing, and space-bearing payloads, an unknown protocol, an unknown content
  kind, and an oversize batch naming the byte count and limit) with the display
  revision still 0. `crates/pi-rs-tui/src/display.rs` gains four unit tests
  proving the kitty and iTerm2 sequences against real rasterized frames, that a
  repainted row re-places the image covering it while an untouched row does not,
  and that a partially clipped image is not placed. `docs/lua-extension-api.md`
  gains an inline-image section and `docs/lua-coding-spine.md` records schema
  version 3. `cargo fmt --all -- --check`, `cargo test --workspace` (76 test
  targets, 323 tests, 0 failures), and `nix flake check` (8 checks: workspace
  tests, clippy, default distribution, raw no-package guidance, file-backed
  application, pi-core package, model-catalog update) pass. **Deliberate
  omissions:** no capability detection is exposed (`terminal_image::
  detect_capabilities` stays internal — the environment a package already reads
  is enough), and no z-order or overlap policy is named.

  **Landed slice (generated API reference) — closes 4.1:** `docs/lua-api-reference.md`
  is the generated inventory of the demonstrated surface, and
  `crates/pi-rs-host/tests/api_reference.rs` is both its generator and its check.
  Nothing in the inventory is typed by hand. Module members come from an ordinary
  package that walks the live `pi` table it receives — the shape
  `public_surface.rs` already pinned, extended to full recursion — so names,
  kinds, and every bound constant's *value* are read out of the running VM; a
  table reached twice is emitted as an alias of the path that reached it first,
  which is how `pi.roots.v1.module` is documented once rather than duplicated.
  Handle methods cannot be walked: mlua protects userdata metatables
  (`getmetatable` returns `false`) and the `debug` library is not opened, so
  `Display:submit` is unreachable from Lua reflection. They are therefore read
  out of the `impl UserData` blocks under `crates/pi-rs-host/src`, bounded by the
  first-column `}` that `cargo fmt` guarantees, and the scan fails loudly on any
  registration form it cannot read rather than skipping it. Prose is curated but
  fail-closed in both directions: a member with no sentence, a sentence with no
  member, a handle type with no section, or a signature that does not open with
  its own generated name fails the test, and a function's parenthesis-or-brace
  form is checked too. Both directions were verified by temporarily adding
  `pi.records.v1.drift_probe` and `RecordStore:drift_method`, each of which
  failed the check by name. Regeneration is
  `PI_RS_WRITE_API_REFERENCE=1 cargo test -p pi-rs-host --test api_reference`;
  without the variable the test diffs the committed file and reports the first
  differing line. The check needs no ambient sibling checkout — it reads only
  this repository's own `crates/pi-rs-host/src` and `docs/` — and it rides the
  existing `workspace-test` Nix check rather than adding a derivation, confirmed
  by `tests/api_reference.rs` appearing in that check's sandbox log. The
  generated file records 8 modules, 100 members, 10 handles, and 35 handle
  methods; `docs/lua-extension-api.md` gains a pointer naming the generated
  reference as authoritative for the inventory while it keeps the mechanism
  rules. `cargo fmt --all -- --check`, `cargo test --workspace` (78 test targets,
  324 tests, 0 failures), and `nix flake check` (8 checks: workspace tests,
  clippy, default distribution, raw no-package guidance, file-backed application,
  pi-core package, model-catalog update) pass.

  **Deliberate omission:** the reference is an inventory, not a semantics
  document. Argument names, refusal rules, and ordering guarantees stay in
  `docs/lua-extension-api.md` and `docs/lua-coding-spine.md`, which remain
  hand-written; only coverage is mechanically enforced. A handle method's
  *behaviour* can still drift from its sentence — only its existence cannot.

After 4.1, `/orchestrate` may run **Wave P1** for the disjoint package trees.

- [x] **4.2 — Config/resource package** (**Wave P1**, path owner:
  `crates/pi-rs-builtins/config/**`; depends on 4.1).

  Implement `config.lua` declarations, package/module selection, themes, keymaps,
  providers/models, tools, resource paths, and root selection. Load XDG first and
  legacy config only as fallback; project configuration has explicit trust policy
  and reload publishes atomically.

  **Accept:** precedence/trust/rollback/idempotence matrices pass; all effective
  configuration is inspectable; replacing the file-backed config changes policy;
  Rust contains no product default.

  **Landed slice (configuration spine, precedence/trust/rollback/idempotence):**
  `crates/pi-rs-builtins/config/` is the ordinary Lua package graph
  (`json.lua`, `paths.lua`, `schema.lua`, `trust.lua`, `defaults.lua`,
  `init.lua`) over the public surface only; no directory name, precedence rule,
  fallback, trust concept, merge rule, or default is in Rust, and no host file
  changed for this slice. Three layers merge in order — shipped `defaults`,
  `user` (canonical `<config>/config.lua`, else legacy `settings.json`), and
  `project` (`<root>/.pi/config.lua`, trusted directories only). A configuration
  file is not a package: it is a chunk loaded with an explicit environment of
  pure libraries (each proxied read-only) with no `pi`, `io`, `os`, `require`,
  or `load`, so capability arrives only by naming packages in `packages`, which
  load through `pi.packages.v1`. `pi.config.schema@1` is the one fail-closed
  schema (unknown key or wrong type is an error naming its dotted path; records
  and maps merge, lists and scalars replace) and it records the layer and file
  behind every dotted leaf. Trust is a durable append-only record under
  `<state>/pi/trust` through `pi.records.v1`; asking a question creates nothing,
  and repeating a decision appends nothing. Publication is atomic: discovery,
  evaluation, validation, merge, and package loading complete before anything
  visible changes, a package already loaded is retained rather than restarted,
  and retired packages are disposed after the swap. `sources()`/`errors()`
  describe the most recent attempt while `effective()`/`provenance()`/
  `revision()` keep describing the last configuration that took effect.
  `init.lua` registers one application event middleware `pi.builtins.config`
  (order `-200`) that composes on the first dispatch, republishes
  `event.config`/`event.config_revision`, sets `event.model` from a catalog row
  when the event carries none, and recomposes only on an explicit
  `config_reload` event. Evidence:
  `crates/pi-rs-builtins/tests/config_package.rs` (15 tests) drives every
  scenario from file-backed packages through the public kernel transaction —
  canonical-over-legacy precedence with the legacy file byte- and
  mtime-unchanged, legacy-only fallback reporting unknown keys, no fall-through
  from a broken canonical file, the trust matrix (undecided, trusted, repeated,
  revoked, other directory) with the on-disk record count asserted, rollback
  across four failure modes with settings/revision/packages intact, idempotent
  recomposition (same revision, same package scope) and generation swap,
  duplicate package refusal, two-directional leaf/provenance coverage, the
  resource matrix, `$HOME` defaults with a refused relative `XDG_*_HOME`, model
  policy changing when the file changes, the sandbox refusing host capability,
  a zero-configuration run that writes nothing (no trust store, no state root),
  and the absence of any host configuration module.
  `docs/lua-config-package.md` documents the package.
  `cargo fmt --all -- --check`, `cargo test --workspace` (78 test targets, 339
  tests, 0 failures), and `nix flake check` (workspace tests, clippy, default
  distribution, raw no-package guidance, file-backed application, pi-core
  package, model-catalog update) pass.

  **Landed slice (applying the declarative sections):**
  `crates/pi-rs-builtins/config/apply.lua` is the second half of the package:
  validating a section proves the file is well formed, applying it makes the
  product behave differently. Two mechanisms carry it and neither is new —
  `modules` resolves every named identity through
  `pi.kernel.v1.module.require`, and `theme`, `keymaps`, and `providers`
  become `pi.kernel.v1.declare` rows (`pi.config.theme`,
  `pi.config.keymap:<binding>`, `pi.config.provider:<name>`, sorted, each
  carrying the `layer` and `origin` file behind it). A configured model starts
  from its reviewed catalog row and takes the section's `api`/`base_url`
  overrides, so no cost, context window, or token budget is invented; a model
  the catalog does not carry is an error naming its dotted path (full custom
  rows are 6.4). `apply.plan()` is pure and runs during composition, so a
  section the product cannot accept rolls the whole reload back instead of
  half applying after publication; module pinning and the declaration swap
  join the same attempt, and `reconcile_generation` now returns the packages
  it added so a later step can undo them.

  Two host behaviours shaped the mechanism and are worth carrying forward.
  First, `pi.kernel.v1.declare` refuses a second declaration of one kind and
  id, and a declaration lives exactly as long as the scope that made it, so
  the configuration package — whose scope outlives every reload — can never
  re-declare its own theme; the staged plan is therefore replayed by a tiny
  package it loads and disposes like any other (`pi.config.declarations`, a
  two-line constant chunk carrying no configuration data), and the ids stay
  stable, which makes the order dispose-then-load with the previous plan put
  back on refusal. Second, `module_api::remove_scope` clears *every* surviving
  module's cached value on any disposal, so the staged plan lives at
  `apply.lua` file scope rather than inside the factory — a factory local
  would be staged onto the instance the disposal just retired. Evidence:
  `crates/pi-rs-builtins/tests/config_package.rs` grows to 19 tests with four
  new rows — declarations read back through `pi.kernel.v1.registered` with
  their layer/origin and catalog-backed model rows, replacement and retraction
  across reloads with exactly one declaration package alive, an unknown
  configured model failing the reload with the published declarations intact,
  and module pinning with its own rollback — plus a zero-configuration
  assertion that nothing is declared and no declaration package is loaded.
  `docs/lua-config-package.md` documents the applier.
  `cargo fmt --all -- --check`, `cargo test --workspace` (78 test targets, 343
  tests, 0 failures), and `nix flake check` (8 checks: workspace tests,
  clippy, default distribution, raw no-package guidance, file-backed
  application, pi-core package, model-catalog update) pass.

  **Landed slice (applying the tools section) — closes 4.2:**
  `crates/pi-rs-builtins/config/tools.lua` (`pi.config.tools@1`) hands
  `tools.root`, `tools.suppress`, and `tools.settings` to the shipped suite
  through `pi.tools.suite@1` and the one tool declaration path
  `pi.agent.tools@1`. The clobber the previous session parked is resolved by
  **ordering, not privilege**: the distribution re-declares the suite from
  `defaults/init.lua` (`pi.builtins.defaults.tool-root`, order `-99`) after this
  package publishes at `-200`, so `init.lua` now registers a *second*
  application event stage, `pi.builtins.config.tools`, at order `-50`. A
  configuration is a higher layer than a distribution default, so it runs last
  and the last stage to run owns the registry; any package wanting the final
  word registers a later stage the same way. The split matches the rest of the
  package: `plan()` validates during composition (so a refusal rolls the whole
  reload back), and `reconcile()` re-declares in the `-50` stage. Re-declaring
  costs one unregister plus one declare, so it happens only when the published
  revision or the launcher root changes — the launcher root is tracked
  precisely because a root change is what makes the distribution's stage
  re-declare and drop the configured suppression. The applied memo lives at
  `tools.lua` file scope for the same reason `apply.lua` stages its plan there:
  disposing any package clears every module's cached value. Refusals are
  fail-closed and name their dotted path: an unknown tool, settings for a tool
  the same file suppresses, a relative `tools.root`, a `name` key (the suite
  retracts a tool by its default name, so a rename would leak a declaration),
  and a `tools` section in a distribution carrying no `pi.tools.suite@1`.
  `pi.config.schema@1` gained one node kind, `scalar` (any string, number, or
  boolean), so `tools.settings.<tool>` carries a tool's own option values
  without making a configuration quote numbers; `settings.tools()` reports the
  live declaration. Evidence:
  `crates/pi-rs-builtins/tests/config_package.rs` grows to 26 tests with seven
  new rows — six carry the shipped tool distribution (agent tool path, the four
  core tools, `defaults/init.lua`) so the real suite meets the real
  distribution stage: a configuration-free run leaving the distribution's
  policy alone, a configured root outranking the launcher root and being handed
  back when the section is removed, suppression disappearing and returning, a
  numeric per-tool setting reaching `read` while the configured root still
  applies, a new launcher root not losing the configured policy, and the four
  refusals each keeping the live declaration; the seventh refuses a `tools`
  section with no suite loaded. The pre-existing provenance row now carries the
  tool distribution too, because its project layer configures `tools`.
  `docs/lua-config-package.md` documents the section, the ordering, and the
  inspection surface. `cargo fmt --all -- --check`, `cargo test --workspace`
  (78 test targets, 350 tests, 0 failures), and `nix flake check` (8 checks:
  workspace tests, clippy, default distribution, raw no-package guidance,
  file-backed application, pi-core package, model-catalog update) pass.

  **Moved to 4.4 (not dropped):** the `roots` section still validates, merges,
  and publishes without acting, and it cannot act from this item's owned paths.
  `kernel_api::resolve_root` picks the highest-priority *active* root per kind,
  a root entry is keyed `kind\0id` and refuses re-registration from another
  source, `DeclarationKind` has no `root` member, and no public Lua surface
  lists, deactivates, or re-prioritises a root — so a configuration can only
  select a root once 4.4 decides how a selected package wins. That decision and
  the acceptance row for it belong to 4.4, which already owns root suppression
  and replacement.

  **Known gaps carried forward:** the package is deliberately **not** in
  `crates/pi-rs-builtins/default.json` — 4.4 extends the declarative manifest
  once and reconciles the overlap with `pi.builtins.defaults.model` (order
  `-100`) and `pi.builtins.defaults.tool-root` (order `-99`), which the new
  `-50` stage now sits after. `tests/README.md` still has no acceptance row for
  `crates/pi-rs-builtins/tests/**` (a pre-existing gap covering the agent,
  frontend, tool, and configuration package suites).

- [x] **4.3 — Configurable session package** (**Wave P1**, path owner:
  `crates/pi-rs-builtins/session/**`; depends on 4.1).

  Implement optional persistent conversation policy over the public record store:
  record schema, reconstruction, names, branch/tree meaning, selection,
  compaction records, retention, and legacy interpretation. Session actions are
  queued and every write targets XDG.

  **Accept:** suppressing the package leaves the useful ephemeral app from 3.6;
  a small file-backed replacement persists a different schema; branch, compact,
  resume, corruption, cancellation, stale-handle, and legacy-read/XDG-write paths
  are covered.

  **Landed (the whole package):**
  `crates/pi-rs-builtins/session/` is an ordinary three-file package graph over
  the public surface only. `records.lua` (`pi.session.records@1`) is pure: it
  owns the record kinds (`header`, `message`, `title`, `model`, `compaction`,
  `branch`, `note`), a 16 KiB per-field text budget, and the left fold that
  reconstructs a conversation. Writing is fail-closed and names its dotted path;
  folding is deliberately tolerant, because a log outlives the package that
  wrote it — an unknown kind, a missing header, or a compaction pointing past
  the end is counted in `diagnostics` instead of raising. `store.lua`
  (`pi.session.store@1`) turns `pi.records.v1` into sessions — id-as-file-name,
  time-ordered generated ids with collision retry, start/open/list/describe,
  prefix-copy branching that re-identifies the copy with a `branch` record,
  compaction, and retention — while still naming no directory: both the write
  destination and the read-only legacy directory are arguments. `init.lua` picks
  them from the one path policy, `pi.config.paths@1` (`sessions` resource:
  `$XDG_STATE_HOME/pi/sessions`, legacy `~/.pi/agent/sessions`), and refuses to
  write at all when that module is absent rather than inventing a second
  directory rule.

  Integration is two public stages and no root, which is what makes suppression
  free: `pi.builtins.session.record` (`agent`/`render`, order `100`) folds the
  settled agent batch into records and returns the batch untouched, so
  persistence can never alter, delay, or fail a turn; `pi.builtins.session.command`
  (`application`/`event`, order `-60`) answers a `session` event by stopping the
  chain and queueing one `session_result` action (`status`, `list`, `describe`,
  `resume`, `new`, `name`, `compact`, `branch`, `retain`, `close`). Recording
  reads only the agent's *public* action vocabulary, so a replacement agent that
  emits the same actions is persisted unchanged. Legacy is read-only in both
  directions: `list` labels every row `canonical` or `legacy`, and `resume` on a
  legacy-only id copies it forward into the canonical directory and continues
  there, leaving the inherited file byte-for-byte as it was — the same promotion
  rule the credential store uses. Retention never removes a legacy log, the live
  log, or a log the listing could not open, so a locked or damaged file is
  diagnosed rather than deleted.

  Evidence: `crates/pi-rs-builtins/tests/session_package.rs` (19 tests) drives
  every scenario through the public kernel transaction — suppression leaving
  3.6's ephemeral application and writing nothing at all, a file-backed
  replacement persisting its own schema at its own destination, XDG-only writes,
  tool settlement with `call_id`/`name`/`ok`, the *shipped* agent under a fixture
  provider proving the recorded vocabulary is real, resume reconstructing and
  continuing the same log, reset ending a log, naming and compaction as appended
  records, a refused impossible compaction, branching with parent provenance,
  retention keeping the live log and sparing legacy, legacy promotion, a torn
  log diagnosed as `partial-write` with recording recovering into a new log, a
  foreign log folding with diagnostics, a stale handle dropped without failing
  the turn, package disposal releasing the store's lock to a second host,
  16 KiB truncation, an unusable state root, and an unknown command refused by
  name. `docs/lua-session-package.md` documents the stages, schema, storage
  rule, commands, the replacement seam, and the cancellation rule below.

  **Cancellation — the last criterion, closed here.** A *kernel-cancelled*
  record operation looked unreachable from Lua, because `pi.records.v1` accepts
  only a kernel `Cancellation` (`crates/pi-rs-host/src/bindings/records.rs`),
  that userdata exposes `is_cancelled`/`wait` but no `cancel`, and a scope token
  is cancelled only by disposal. It *is* reachable, through the public surface
  alone: a package captures its own dispatch cancellation with
  `pi.kernel.v1.cancellation()` and publishes it as an ordinary module value; a
  second package requires that module, the host disposes the publisher (which
  cancels its scope token), and the survivor still holds a live, now-cancelled
  kernel token.
  `crates/pi-rs-host/tests/records_store.rs::a_cancelled_kernel_token_refuses_record_work_before_it_starts`
  drives exactly that journey: `append`, `copy`, and `cursor:next` each fail
  with `record-store operation cancelled` *before* any blocking work, the log
  keeps exactly its settled records with no torn line, no copy destination is
  published, an uncancelled append still lands (the refusal is per operation,
  not per handle), and a cancellation the kernel did not issue
  (`pi.effects.v1.cancellation.new()`) is refused by name.

  The evidence sits in 4.1's owned binding path rather than the session suite
  on purpose, and this session claimed it serially with no orchestrated batch
  active: the session package supplies no cancellation of its own, so every
  session write inherits the ambient dispatch token, and each call site is
  already `pcall`-wrapped — a refusal becomes a `session_result.error`
  diagnostic on the same branch as the covered stale-handle path, which is why
  duplicating it under `crates/pi-rs-builtins/session/**` would assert nothing
  new. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`
  (one pre-existing `double_ended_iterator_last` warning in the `agent_package`
  test, outside the flake's `--lib --bins` clippy gate), `cargo test --workspace`
  (80 test targets, 370 tests, 0 failures), and `nix flake check` (8 checks) all
  pass on the closing change.

  **Carried forward to 4.4 (not dropped):** (a) the package is deliberately
  **not** in `crates/pi-rs-builtins/default.json` or `flake.nix`'s
  `mkPiPackages` copy list, for the same reason the configuration package is
  not: 4.4 extends the declarative manifest once, and it must add `config/` and
  `session/` together because `session/init.lua` requires `pi.config.paths@1`.
  (b) A resumed log reconstructs its conversation, but nothing hands it back to
  the *live* shipped agent: `pi.agent.turn@1` keeps `conversation` private and
  has no public `restore` event, and `crates/pi-rs-builtins/agent/**` is not this
  item's owned path. 4.4 owns that integration and should decide it as an agent
  event rather than a session-package reach-in. (c) `agent_message` publishes
  the settled text and a tool-call count, not the provider content blocks, so a
  persisted assistant turn carries text only; exact provider replay would need
  the agent to publish the settled message. (d) `tests/README.md` still has no
  acceptance row for `crates/pi-rs-builtins/tests/**`, now covering the agent,
  frontend, tool, configuration, and session package suites.

- [x] **4.4 — Integrate configuration/session and close replacement composition**
  (**serial after Wave P1**).

  Extend the declarative default manifest once, then independently suppress and
  replace application, agent, frontend, and session roots. Compose representative
  event/render middleware and config declarations. Prove deterministic conflicts,
  module versions, lifecycle cleanup, reload rollback, watchdog isolation, and
  copied-to-disk reproduction across the expanded graph.

  Inherited from 4.2 and **closed by the second landed slice below**: make the
  configuration's `roots` section act. Selecting a root from
  `<config>/config.lua` needed a mechanism the host did not have — it resolved
  a root by highest priority among active registrations and exposed no Lua way
  to list or override that — so the section validated and published without
  changing anything. `pi.roots.v1.list`/`select` and `pi.config.roots@1` are
  that mechanism and its policy.

  **Accept:** `nix run` remains input-ready with and without persistent sessions;
  each package/root is suppressible or replaceable; two extensions compose without
  privileged ordering; zero-builtin/file-backed checks remain green.

  **Landed slice (the manifest extension, and persistence made optional in
  fact):** the shipped index now carries the whole product.
  `crates/pi-rs-builtins/default.json` gained eleven entries — the eight
  configuration modules first (nothing requires them, and `config/init.lua`
  requires the other seven) and the three session modules between the
  application coordinator and `defaults/init.lua` — and `flake.nix`'s
  `mkPiPackages` copies both trees, so `nix run` ships them. Load order only
  satisfies `module.require` at load time; what runs when is decided by
  middleware `order`, which is why the configuration's `-200` model stage wins
  over the distribution's `-100` candidate list and its `-50` tool stage wins
  over `-99`, with no privilege anywhere.

  Shipping the session package exposed one real integration bug, fixed here in
  `crates/pi-rs-builtins/session/init.lua`: the distribution configures a model
  on *every* startup, and `agent_configured` was a recordable step, so a bare
  launch created a durable log (header + `model`) and its lock before anything
  was said — 20 launches left 16 files. A `model` record is now **deferred**:
  `steps_for` marks it, `record_batch` holds it at file scope, and it is
  appended immediately before the first real record instead of starting a log.
  A conversation is therefore persisted in exactly the previous order (the 19
  session tests are unchanged and green), while a launch that says nothing
  writes nothing at all. `store.start` already carried `model_id` into the
  header, so nothing is lost when a deferred record is dropped by a reset.

  Evidence: `crates/pi-rs-app/tests/default_distribution.rs` is the manifest's
  acceptance owner and grew to 10 tests. Its harness became hermetic first —
  a `Sandbox` pins `HOME` and all four `XDG_*_HOME` variables for both the
  subprocess launcher and the in-process host, because a distribution that now
  reads `<config>/pi/config.lua` and writes `<state>/pi/sessions` would
  otherwise read the developer's configuration and write their sessions. New
  rows: startup writes no session log and creates no state root; a user
  `config.lua` selecting `openai/gpt-5.1` outranks the distribution default in
  the rendered header; an index without the session tree renders a
  byte-identical first frame; a real offline turn persists exactly one log
  under the canonical XDG entry with
  `header, model, message×4` and answers `session status` from inside the
  distribution; and the same turn with the session tree removed still
  completes, writes nothing, and leaves the `session` command unanswered.
  The tree-equality assertion now covers `config` and `session`, so neither
  can drift out of the index. The Nix `default-distribution` check gained the
  same three claims at distribution level (no state root after startup, a
  `config.lua` changing the header, and a session-free manifest diffing equal).
  `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (8
  pre-existing warnings, none in the touched files), `cargo test --workspace`
  (80 test targets, 374 tests, 0 failures), `nix flake check` (9 checks), and
  `nix run .` under a private `HOME` (input-ready, nothing written) all pass.

  **Landed slice (the `roots` section acts: selection instead of outbidding).**
  A root kind resolved to the highest-priority *active* registration and failed
  on a tie, so replacing the shipped frontend meant outbidding it and a
  configuration naming one could change nothing. The kernel now carries a
  per-kind **selection** beside the registrations
  (`ROOT_SELECTIONS_KEY` in `crates/pi-rs-host/src/kernel_api.rs`):
  `pi.kernel.v1.roots([kind])` reports registrations as data (`kind`, `id`,
  `source`, `priority`, `active`, `selected` — never the `dispatch` function,
  so listing grants no ability to run one), and
  `pi.kernel.v1.select_root(kind[, id])` names the row a kind resolves to.
  `pi.roots.v1.list`/`select` are the facade, restricted to exactly the kinds
  `register` accepts, so a selection that validates can always be applied.
  A selection outranks priority entirely and cannot revive an absent or
  inactive registration — a stale one fails the next dispatch of that kind with
  `HostError::UnknownSelectedRoot` (kind, id, selecting source) rather than
  silently falling back to the bidding. It is owned like a registration: a
  second source selecting the same kind is the same deterministic conflict, and
  `remove_scope` drops selections of the disposed scope, so disposal or reload
  rollback restores priority resolution instead of leaving a dangling choice.

  Policy stays in Lua. `crates/pi-rs-builtins/config/roots.lua`
  (`pi.config.roots@1`, eleventh entry in `default.json`) is the same
  `plan`/`reconcile` split `pi.config.tools@1` uses, with one deliberate
  difference: **`plan` runs after the configuration's own `packages` load**,
  inside `reload` beside `apply.pin`, because the usual reason to name a root
  is that one of those packages registers it. An id no active registration
  carries fails the whole reload, rolls the loaded generation back, and lists
  the ids that do exist. `reconcile` is a third application event stage,
  `pi.builtins.config.roots` at order `-49`, a comparison first and an action
  second; removing a kind from the section clears it rather than freezing the
  last answer. `roots.session` is answered with the replacement path (package
  index or `packages`) instead of applied: the shipped session package is two
  middleware stages and registers no root.

  Evidence: `crates/pi-rs-host/tests/root_selection.rs` (10 tests, new) drives
  the mechanism through the public surface only — a named root beating a
  higher-priority registration, listing without the dispatch handle, a stale
  and an inactive selection failing the dispatch by name, a second source
  refused, the owner reselecting and clearing, disposal restoring priority, a
  blank id refused, an unregisterable kind refused by both `list` and `select`,
  and a priority tie decided by selection instead of renumbering.
  `crates/pi-rs-builtins/tests/config_package.rs` grew to 31 tests with the
  configuration journey: a config-loaded package's root selected over a
  priority-10 shipped root, the section removed (including the one-dispatch lag
  that follows from the host resolving a root *before* that dispatch's
  middleware), an unknown id rolling the reload back and disposing what it
  loaded, `roots.session` diagnosed, and no selection at all without the
  section. `docs/lua-config-package.md` gains a `Roots` section and
  `root_selection()`; `docs/lua-extension-api.md` documents list/select; the
  generated `docs/lua-api-reference.md` and the pinned `public_surface`
  key list cover the four new members. `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets` (pre-existing warnings only; the one
  `pi-rs-host` lib warning is the untouched `register_root` collapsible `if`),
  `cargo test --workspace` (80 test targets, 389 tests, 0 failures),
  `nix flake check --print-build-logs` (6 checks, all passed), and `nix run .`
  under a private `HOME` (input-ready frame, nothing written) all pass.
  Note for the next session: a new Lua package file must be `git add`ed before
  any Nix check runs — the flake sees only tracked files, so an untracked
  `config/roots.lua` failed `default-distribution` with "package 8 is absent".

  **Landed slice (replacement proven at distribution level, on the agent and
  on the installed binary).** The two earlier slices proved replacement at
  package level; a distribution is where it has to hold, because the shipped
  index registers a root for every kind. Two claims landed, both by *naming*
  a root rather than outbidding one — each replacement registers at priority
  `-10`, below the shipped `0`, so priority resolution alone would keep the
  shipped root.

  `crates/pi-rs-app/tests/default_distribution.rs` (12 tests) replaces the
  **agent**. The whole shipped index is loaded, and an ordinary
  `<config>/pi/config.lua` loads one file-backed package from the canonical
  packages resource (`<data>/pi/packages/agent.lua`) and names its root with
  `roots = { agent = "acceptance.agent" }`. The replacement is 20 lines with
  no provider, no tool loop, and no module shared with the shipped agent: it
  answers `configure` and `prompt` — the two events the shipped application
  coordinator dispatches — with `agent_configured`, `agent_turn_start`,
  `agent_message`, and `agent_status`. The rest of the distribution keeps
  working over it unchanged: the shipped frontend renders its own chrome, user
  row, assistant row, and idle status, and the shipped session package folds
  the replacement's batch into `header, model, message, message` under the
  canonical XDG state entry and answers `session status` with two messages —
  the `agent`/`render` middleware stage reads the public action vocabulary and
  never asks which root produced it. The control test is the same package
  loaded by the same configuration with only `roots.agent` removed: the
  shipped agent runs the turn (fixture `401`, credential guidance rendered)
  and the replacement never appears, so nothing above is explained by
  registration order or priority. The harness change that made this possible
  is small: `Sandbox` gained `write_package`, and `Distribution::from_packages`
  now takes the sandbox, because a configuration must exist on disk *before*
  the host loads the index.

  The Nix `default-distribution` check makes the same claim about the
  installed binary and a root the launcher itself dispatches: `pi` with no
  arguments, a `config.lua` naming a replacement **frontend** root, and the
  startup frame is the replacement's output with the shipped footer
  (`enter send`) absent. That closes the gap the old block left — it only
  overrode the application root through an explicit `--package`, which is a
  command-line selection rather than a configured one.

  Checks: `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets` (pre-existing warnings only; none in the touched files),
  `cargo test --workspace` (80 test targets, 391 tests, 0 failures),
  `nix flake check --print-build-logs` (6 checks, all passed),
  `nix build --rebuild .#checks.x86_64-linux.default-distribution` (re-ran the
  new block rather than reusing the cached output), and `nix run .` under a
  private `HOME` (input-ready frame, no state root written).

  **Landed slice (composition across the expanded graph — the last acceptance
  criterion, closed here).** Every earlier slice proved *replacement*; what was
  never asserted is that two packages nobody shipped compose over the whole
  distribution, and that the shipped packages hold no privileged position in
  that composition. `crates/pi-rs-app/tests/default_distribution.rs` grew from
  12 to 21 tests, all through the public surface: a `config.lua` in the pinned
  `$XDG_CONFIG_HOME` naming file-backed packages under the canonical
  `$XDG_DATA_HOME/pi/packages` resource, over the *whole* shipped index.

  Two extensions, each one `agent`/`render` stage that marks the settled
  assistant message, compose in `order` (`[a]` then `[b]`), and the shipped
  session package's own recording stage at order `100` records what they
  produced — it reads the public action vocabulary and never asks which source
  emitted an action. Swapping *only* the two `order` numbers, with the same
  files, names, and `packages` order, flips the composition, so nothing about
  load order, file name, or provenance decides the chain. A third extension at
  order `300` runs after the shipped `100`: its mark reaches the frame while
  the persisted record keeps the untransformed text, which is the sharpest
  statement that a shipped stage is ordered rather than final.

  The rest of the matrix, at distribution level rather than on a bare kernel:
  **deterministic conflict** — two sources claiming one `kind`/`phase`/`id` are
  refused in *either* declared order, and because the refusal rolls the whole
  reload back, the extension that had already loaded composes no more either;
  **module versions** — an extension requiring `pi.config.paths@1` receives the
  real shipped path policy (it refuses to load unless the module resolves a
  `sessions` destination), while `@2` fails that package and rolls the reload
  back, the same pair proving the source is valid; **lifecycle cleanup** —
  dropping one entry from `packages` and dispatching `config_reload` disposes
  exactly that package and drops its stage, while the retained package is kept
  rather than reloaded and keeps its place; **reload rollback** — a package
  that raises at load leaves the previous generation composing unchanged;
  **watchdog isolation** — a stage that never returns costs one refused
  dispatch under a 400 ms budget, after which the shipped session command still
  answers and the shipped frontend still repaints. Because the runaway sits
  under a *nested* root dispatch (the coordinator asking the agent root), the
  watchdog's stop arrives as a Lua error carrying the traceback, not the
  host-level `Timeout` a top-level dispatch returns; both are the same bound.

  **Simultaneous independence** closed with it: one configuration replaces the
  `agent` and `frontend` roots at once (both registered at priority `-10`,
  below the shipped `0`, so only being *named* resolves them), the replacement
  frontend renders the replacement agent's message, the shipped footer never
  paints, and the shipped session package still folds the turn into
  `header, model, message, message` and answers `session status`.

  Two harness additions were needed and are small: a `plain` fixture that
  settles without deltas (the shipped transcript deliberately keeps an
  already-streamed row as streamed, so a render transform is only observable in
  the frame on a non-streamed turn), and an explicit dispatch-timeout
  constructor plus a non-panicking `try_dispatch` for the watchdog budget.

  This work also corrected `docs/default-distribution.md`, which claimed a
  shipped stage could be replaced by registering the same `kind`/`phase`/`id`
  from your own package. The host refuses exactly that as a conflict; the doc
  now says so and gains a `Composing extensions` section for the rules above.

  Checks: `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets` (8 pre-existing warnings, none in the touched file),
  `cargo test --workspace` (80 test targets, 400 tests, 0 failures),
  `nix flake check --print-build-logs` (6 checks, all passed), and `nix run .`
  under a private `HOME`/XDG set (input-ready frame, nothing written).

  **Standing notes carried past 4.4 (deliberate, not dropped):**
  1. `tests/README.md` still has no acceptance row for
     `crates/pi-rs-builtins/tests/**` (agent, frontend, tool, configuration,
     and session package suites). Left alone again: that file has unrelated
     uncommitted user edits, and touching it would mix them into a commit.
  2. `pi.roots.v1` registers, lists, and selects `application`, `agent`, and
     `frontend` only, while `RootKind` and `DESIGN.md` also name `session`. A
     session root is therefore registrable only through `pi.kernel.v1.root`.
     Left as is: nothing dispatches a session root today, and widening the
     facade would ship a public kind with no consumer.

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
