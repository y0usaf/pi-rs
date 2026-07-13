# pi-rs design

pi-rs is a minimal, high-performance Rust coding harness whose installed
executable is `pi`. Its shipped coding product is ordinary Lua 5.4 policy over
a small Rust mechanism kernel. The default experience deliberately resembles
Pi in a bounded set of interactions; pi-rs is not a faithful port of Pi.

`DESIGN.md` is the product and architecture contract. `PLAN.md` is the ordered
implementation plan. If they disagree, stop and reconcile them before changing
implementation. The former port remains on `pi-rust-rewrite`; it is historical
provenance and a source of already-proven mechanism ideas, not the specification
for `main`.

## Product contract

pi-rs provides:

- a fast terminal coding harness with a restrained default workflow;
- application, agent, frontend, and optional session policy authored in Lua;
- shipped defaults loaded through the same public package/module/declaration
  paths as file-backed user code;
- immutable snapshots and read handles into Lua, with typed actions and effects
  returned or queued out;
- generic terminal, OS, provider/auth, and durable-record mechanisms in Rust;
- useful zero-builtin operation through an ordinary file-backed Lua application.

Embedding is distribution, not privilege. An embedded package receives the
same capabilities, watchdog, lifecycle, conflict rules, and declaration paths
as the same package loaded from disk. Synthetic source identity records only
provenance.

### Bounded Pi experience reference

Pi v0.79.0 at `ref/pi` commit
`c5582102f51b143fadc05180e0f8aed050e923b3` is a focused observation source for
the checked canonical experience set only:

- transcript rhythm, spacing, color, wrapping, and tool presentation;
- prompt editing, completion, keybindings, paste, and external-editor flow;
- streaming/thinking presentation and cancellation;
- steering and follow-up queues;
- representative selectors, dialogs, notifications, status, header, and footer;
- the restrained default coding journey joining those states.

The compact grid/input fixtures selected by PLAN 0.2 define this set. A named
canonical frame may be cell-exact and a named input trace may be transition-
exact. Unrecorded Pi behavior does not become a requirement by adjacency. Pi is
not an oracle for pi-rs CLI breadth, errors, tools, sessions, configuration,
package management, extension APIs, internal state machines, or historical edge
cases.

### Exhaustive provider/auth compatibility subsystem

Provider and authentication behavior is the sole exhaustive Pi compatibility
promise. Against the same pinned revision, pi-rs preserves every provider/model
row and advertised API family in the supported catalog, including:

- Anthropic Messages, OpenAI Completions, OpenAI Responses, Azure OpenAI
  Responses, OpenAI Codex Responses, Google Generative AI, Google Vertex,
  Mistral Conversations, and Bedrock Converse Stream;
- request conversion, required headers, streaming event conversion, usage/cost
  accounting, provider-visible errors, cancellation, and protocol-specific
  options;
- catalog lookup, provider/model identity, capability metadata, and dispatch to
  the advertised protocol family;
- API-key resolution from runtime, stored, environment, configuration, and
  command-backed sources with deterministic precedence, refresh, locking, and
  redaction;
- Anthropic Claude Pro/Max, GitHub Copilot, and OpenAI/Codex subscription OAuth,
  including browser/PKCE or device-code login as applicable, callback/headless
  outcomes, refresh, expiry, cancellation, and logout.

Closure requires a fail-closed catalog/API inventory and deterministic wire/auth
fixtures: no unexplained supported row, protocol, credential source, or
subscription flow may remain. Shared HTTP, retry, SSE, PKCE, and device-code
machinery is implemented once; brands are data unless a wire protocol truly
differs.

This promise ends at the subsystem boundary. Provider selection, configuration
presentation, login UI, commands, and surrounding agent workflow are replaceable
Lua product policy. Their appearance and sequencing need not match Pi unless a
specific state is also in the canonical experience set.

### Explicit non-goals

pi-rs does not promise:

- whole-product behavioral or data compatibility with Pi;
- Pi's TypeScript extension API, Node runtime, npm execution model, complete CLI,
  package manager, JSON/RPC mode, HTML export, or every external extension;
- exact Pi errors, sessions, settings formats, request ordering outside the
  provider/auth subsystem, or undocumented edge cases;
- a public registry for every private Lua helper;
- performance merely because the kernel is written in Rust.

An omitted product feature returns only for an independent pi-rs use case. It
must then be Lua policy through an existing public seam or a generic mechanism
that justifies a new seam.

## Mechanism and policy boundary

Rust owns mechanism; Lua owns product decisions. The following placements are
locked until this table is deliberately amended.

| Area | Rust mechanism | Lua policy | Hot-path guard |
|---|---|---|---|
| Runtime and packages | Lua VM, versioned module loader, source-neutral capabilities, transaction publication, watchdogs, generation-safe handles, scoped disposal | package graph, root selection, product composition | One bounded host entry per dispatch/resume; no mutable host borrow crosses it. |
| State changes | immutable snapshot construction, action/effect validation, deterministic apply queue, cancellation | application/agent/frontend/session reducers and action choice | Snapshots are bounded views or read handles; action batches cross once per transition. |
| Terminal/display | byte decoding, Unicode cells, width/wrap/layout/clipping primitives, image primitives, differential ANSI output | component tree, editor behavior, transcript/chrome appearance, focus/keymaps | Lua submits retained trees/display lists in batches; never one callback per byte, cell, escape, or style run. |
| Async OS effects | filesystem, process trees, HTTP/SSE/WebSocket, timers, clipboard, crypto, timeouts, backpressure | tools, retries, truncation choices, workflow, user messaging | Streams cross as bounded chunks/events; every task/resource is cancellable and scope-owned. |
| Provider/auth | wire protocols, shared transport, stream conversion, credential storage mechanics, PKCE/device-code engines | provider/model declarations and selection, custom endpoints, login UI | Protocol parsing stays native and incremental; no Lua callback per transport byte. |
| Persistence | generic versioned append-only JSON-value records, atomic append/copy, cursors, locks, corruption reporting | record schema, conversation reconstruction, naming, branching, compaction, retention | Reads use cursors/bounded windows; no full-history copy on each dispatch. |
| Product behavior | generic root/declaration registries only | all application, agent, frontend, tool, command, theme, config, resource, and session behavior | Native acceleration may execute Lua-authored structures but may not choose visible or workflow policy. |

Cross-cutting mechanisms—transport, retry primitives, cancellation, atomic file
replacement, text/display diffing, and resource cleanup—have one implementation.
A native hot path is not an extension-first exception: Lua still authors the
state and policy, while Rust performs a generic batched operation. Any proposal
that moves a product choice into Rust requires a new locked decision, evidence
that batching cannot solve the measured problem, and an ablation-safe public
boundary.

Every uninterrupted Lua dispatch is watchdog-bounded. Awaiting a host future
holds no mutable host state; a resumed coroutine receives a fresh bounded entry.
Failed package loads publish nothing. Stale handles fail after root/session
replacement or reload. Shutdown and reload cancel all work owned by the disposed
scope.

## Public Lua product model

The public surface favors a few stable, broad seams:

- independently replaceable application, agent, frontend, and session roots;
- composable declarations for tools, commands, providers, events, renderers, UI
  slots, themes, and keymaps;
- ordinary versioned Lua modules for reusable and private helpers;
- one declaration mechanism for every repeated kind.

Shipped policy lives in a dedicated builtins package graph, not concatenated
mega-chunks and not Rust branches. A unit is public when it is a replaceable root
or composable declaration; private helper functions and inert package data do
not need ceremonial registration.

Lua reads immutable snapshots/read handles and returns or queues typed actions.
It never receives `&mut` host state. Actions publish only after a successful
bounded dispatch, through one validation/application path.

## Session model

Persistent sessions are optional, configurable Lua policy over the generic Rust
record store. The shipped session package owns:

- its versioned record schema and context-reconstruction reducer;
- creation, naming, selection, branch/tree meaning, and retention;
- compaction records and the decision of what enters model context;
- legacy session parsing, when supported, and all user-facing diagnostics.

The Rust store knows none of those concepts. It guarantees atomic durable
append, locking, iteration/cursors, copy primitives, cancellation, and
corruption errors for arbitrary JSON values.

Suppressing the session package leaves a useful ephemeral conversation. A
file-backed replacement may persist a different schema without private APIs or
frontend forks. Session switches and reloads are snapshot/action transactions;
stale runtime handles cannot mutate the replacement session.

## Storage contract

Canonical user roots follow XDG:

| Class | Root | Typical contents |
|---|---|---|
| Configuration | `${XDG_CONFIG_HOME:-$HOME/.config}/pi` | `config.lua`, declarative product/package selection |
| Data | `${XDG_DATA_HOME:-$HOME/.local/share}/pi` | installed Lua packages and user-authored resources |
| State | `${XDG_STATE_HOME:-$HOME/.local/state}/pi` | credentials, session records, trust and other durable mutable state |
| Cache | `${XDG_CACHE_HOME:-$HOME/.cache}/pi` | safely regenerable downloads and derived data |

An explicit XDG variable wins for its class. If it is absent, the documented
`$HOME` default is used. A missing/invalid required root is a diagnostic, never
an implicit current-directory path.

`~/.pi/agent` is a read-only compatibility fallback, resolved **per resource**:

1. Use the canonical XDG resource when it exists.
2. Only when that individual resource is absent, try its documented legacy
   counterpart.
3. Never merge canonical and legacy copies, and never fall through because a
   present canonical file is malformed or unreadable.
4. Every create/update/delete targets XDG. A mutation after a legacy read creates
   or updates the canonical resource and leaves the legacy source byte-unchanged.
5. Never rename, delete, rewrite, or silently migrate a legacy resource. Any
   import is explicit, reports source and destination, and remains safe to rerun.

This fallback is a storage/provenance rule, not whole-data-format compatibility.
For example, provider credentials are exhaustively supported at this boundary,
while any legacy session interpretation belongs to the selected Lua session
package.

## Performance contract

Performance claims come only from the reproducible release-mode harness created
by PLAN 0.2. It records the reference machine/environment, fixture revision,
sample count, warm/cold conditions, and machine-readable results. Numeric
baselines and budgets are measured there before being written into this section;
0.1 does not invent aspirational thresholds.

| Metric | Required measurement | Goal |
|---|---|---|
| Startup | offline warm-cache process start → first input-ready frame, p50/p95 in ms | At or below the checked release budget. |
| Idle memory | RSS after a fixed settle period with the canonical empty session, MiB | At or below the checked release budget; no upward drift while idle. |
| Input latency | injected key/input timestamp → corresponding flushed frame, p50/p95 in ms | At or below the checked interactive budget. |
| Render cost | unchanged and canonical changed retained-tree frames, p50/p95 µs plus frames/s | At or below cost / at or above throughput budgets. |
| Lua dispatch | snapshot → no-op/action-batch completion, p50/p95 µs and bytes copied | At or below dispatch and copy budgets. |
| Effect round trip | queued local deterministic effect → completion action, p50/p95 µs | At or below the checked effect budget. |
| Release size | stripped closure/binary bytes, reported separately | At or below the checked size budget. |
| Cleanup | live tasks/processes/sockets and RSS growth after repeated cancel/reload/shutdown | Zero leaked resources; bounded growth within the checked tolerance. |

A result without the prescribed harness and environment is diagnostic, not
acceptance. Optimization follows profiles: batch crossings, retain display
structures, bound history/snapshots, and remove unused dependencies. Visual or
provider/auth correctness is never traded for an unmeasured speed claim.

## Doctrine conformance

| Doctrine | Status | Local decision |
|---|---|---|
| 01 extension-first core | follows | Every shipped product feature is an ordinary Lua builtin package using the public surface. Rust contains only mechanisms; zero-pack and replacement tests enforce the split. |
| 02 snapshot in, actions out | follows | Lua receives immutable snapshots/generation-safe read handles and emits validated queued actions. Every dispatch is watchdog-bounded; async resources are scoped and cancellable. |
| 03 state-owning daemon, thin client | deferred | The current product is single-process and no live state must outlive its viewer; durable data lives in files. Detachable/multi-viewer sessions would trigger a separate daemon + versioned-wire design, not hidden coupling now. |
| 04 declarative front, idempotent executor | not applicable | pi-rs is not an activation-time system configuration executor. Lua product declarations and atomic reload are runtime extension concerns, not a Nix-to-manifest appliance. |
| 05 one declaration mechanism | follows | Builtins and users declare each repeated kind through one registry/path; no hand-wired product exception. Singular mechanisms are not forced into registries. |
| 06 bare core must boot | follows | With no builtins/config/extensions, the kernel can load a file-backed Lua application, accept input, render, run an effect, and exit; missing/broken product packages diagnose usefully. |
| 07 Nix as source of truth | follows | The flake owns builds and acceptance. `cargo fmt`/`cargo clippy` are sanctioned direct exceptions; other Cargo commands are iteration aids only. |

## Acceptance and evidence

Permanent evidence is intentionally small and separately owned:

1. Rust mechanism invariants, cancellation, bounds, and cleanup;
2. file-backed Lua capability, source neutrality, replacement, and ablation;
3. compact canonical Pi-derived terminal grids/input journeys;
4. exhaustive pinned provider protocol/model/auth differential fixtures;
5. XDG precedence and read-only legacy fallback matrices;
6. optional/replaced Lua session policy over arbitrary records;
7. release performance budgets and leak checks.

Use Pi only for evidence categories 3 and 4, and only for the question named by
the PLAN item. Generic mechanisms are tested against their own contracts;
public capability is proven by ordinary file-backed consumers. Evidence that no
longer protects one unique contract is deleted. Normal checks consume reviewed,
checked fixtures and do not require an ambient sibling repository.

The release gate is `nix flake check` plus release `nix build`/`nix run` on a
clean checkout. The implementation must also pass zero-pack, per-package,
whole-root replacement, stale-handle, watchdog, cancellation, cleanup, XDG,
focused experience, provider/auth, and measured-budget checks described above.
