# Prime Agent on pi-rs: build plan

## Objective and constraints

Build the Prime Agent value layer on the existing pi-rs kernel without changing the settled split: Rust supplies bounded mechanisms; Lua supplies product policy. The shipped product must be composed from the same public packages and root declarations available to third parties. Every Lua dispatch receives an immutable snapshot, returns typed actions, and runs under a watchdog. The bare core must continue to boot without `pi-rs-builtins` or the Prime Agent packages.

The compatibility target should be pinned before implementation begins:

- the current pi-rs provider/auth target remains Pi v0.79.0 and its existing oracle corpus;
- one exact Prime Agent commit becomes the behavioral reference for the value layer;
- the existing Prime Agent product is the specification for the RLM loop; phi may inform test technique, but neither its design nor its code is imported;
- wire formats, durable-record schemas, Python protocol, and experience fixtures carry explicit integer versions from their first committed form.

The smallest useful vertical slice is: a scripted model asks the persistent Python kernel to mutate state, a second model step observes that state, and the turn stops with prose. The first four phases drive directly toward that slice. Do not start daemon or harness UI work before it passes.

## Verification conventions

All committed gates are exposed through `flake.nix`/`checks` and run with Nix. Native focused commands are useful while iterating, but a phase is not complete until the quoted `nix build` or `nix flake check` command exits zero. Use golden files only at stable boundaries:

| Layer | Parity method | What is compared |
|---|---|---|
| Provider/auth | Existing oracle fixtures | requests, normalized events/errors, auth behavior against pinned Pi v0.79.0 |
| Python bridge and RLM loop | Scripted-LM + REPL-trace | ordered model events, Python requests/responses, tool calls/results, compaction, stop reason, final transcript/state |
| Harness, subagents, daemon, UI, commands | Canonical-experience fixtures | versioned input/event/action/session traces and terminal snapshots where presentation matters |

Every fixture records the reference commit and fixture schema version. Normalize timestamps, IDs, paths, ANSI capabilities, and scheduling jitter; do not normalize ordering, stop reasons, errors, or user-visible state. Add a fixture only after establishing the reference behavior with the pinned TypeScript Prime Agent. A fixture is not permission to copy TypeScript implementation structure into Rust.

## Phase 0 — freeze the contract and make the repository ready

**Dependencies:** none.

### Scope

Create pi-rs `DESIGN.md` before implementation. Record the mechanism/policy boundary, root/package extension boundary, watchdog location, generic record ownership, daemon-owned state, Python-process lifetime, protocol-version policy, and the condition for introducing any new crate. Include the required `Locked decisions`, `Architecture`, `Deferred`, and `Roadmap` sections. Pin the Prime Agent reference commit and inventory the exact behaviors to port, organized as executable fixture manifests rather than a prose wish list.

Capture a small seed corpus from the TypeScript product:

1. RLM prose-only stop;
2. one Python execution followed by model continuation;
3. Python state persisting across two executions;
4. Python exception and timeout;
5. one compaction boundary;
6. one `/refine` mutation;
7. one child/message lifecycle;
8. daemon detach/reattach;
9. heartbeat, schedule, goal, and autonomous-mode examples.

Add empty or initially failing named Nix checks for the future suites so the intended verification surface is visible. Preserve the existing provider/auth checks unchanged.

### Rust mechanism

No product implementation. Only test harness plumbing, fixture schemas/loaders where unavoidable, and Nix check declarations. Decide whether the daemon can live in `pi-rs-app`; create a `pi-rs-daemon` crate only when Phase 7 proves the supervisor has a coherent public boundary.

### Lua policy

No implementation. Define the public package/root names and action/event vocabulary that fixtures will exercise (application, agent, frontend, middleware, and later child/session policy).

### Go/no-go gate

Run:

```sh
rg '^## (Locked decisions|Architecture|Deferred|Roadmap)' DESIGN.md
nix flake check
```

Go only when the first command returns exactly four hits, existing pi-rs checks still pass, the reference commits and fixture schema versions are recorded, and the walking-skeleton plus provider/auth oracle checks remain green.

## Phase 1 — specify typed boundaries before behavior

**Dependencies:** Phase 0.

### Scope

Define the minimum cross-boundary data contracts required by the first vertical slice. Keep them data, not callbacks:

- Python kernel lifecycle and execution requests;
- streamed stdout/stderr, display payload, typed host request, completion, exception, cancellation, and process-exit events;
- agent snapshot fields and actions for requesting model inference, Python execution, compaction, transcript updates, and stop;
- correlation IDs, deadlines, output/byte limits, and protocol versions;
- durable record keys/envelopes and optimistic version checks, reusing the existing generic store rather than adding a harness-specific Rust database.

Put shared serializable types in `pi-rs-ai-types` only when they are genuinely provider-independent public data. Host effects belong in `pi-rs-host`; session records and migrations belong in `pi-rs-session`. Do not place agent-loop decisions in `pi-rs-app`.

Build a conformance harness that can replay a JSON/JSONL scripted LM and record one normalized event/action trace. Fixtures should be readable by the TypeScript reference capture tool and the Rust implementation.

### Rust mechanism

Add enums/structs, validation, bounded codecs, cancellation/deadline plumbing, and trace recorder/replayer. Invalid versions, unknown required fields, duplicate terminal events, oversized frames, and mismatched correlation IDs must fail loudly.

### Lua policy

Add only adapter declarations demonstrating that a replaceable Lua root can observe the new snapshot data and return each new typed action. No RLM state machine yet.

### Go/no-go gate

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).prime-contracts
nix flake check
```

Go only when round-trip/property cases cover every message variant, malformed/oversized inputs are rejected, cancellation reaches the effect boundary, and an external file-backed Lua package—not a privileged builtin—can request a fake Python effect and consume its fake result under a watchdog.

## Phase 2 — hard mechanism I: persistent Python/IPython bridge

**Dependencies:** Phase 1. This deliberately precedes the RLM integration and everything that assumes model-visible computation.

### Scope

Implement a persistent Python subprocess owned as a host resource, with one logical kernel per agent/session identity. Prefer the standard IPython/Jupyter kernel protocol if it provides the required persistence and interrupt semantics; otherwise use a tiny versioned framed helper protocol. In either case, pin the Python environment in Nix and make the helper replaceable/configurable. “IPython compatibility” must be an executable contract, not an in-process Rust Python embedding.

Required semantics:

- variables and imports persist between cells;
- ordered stdout, stderr, rich display, final value, and exception reporting;
- model-generated Python can issue a typed host request and await its typed response without gaining direct mutable host handles;
- interrupt, execution timeout, startup timeout, crash detection, restart policy, and explicit shutdown;
- bounded frame size, output size, display size, and queued requests;
- no accidental inheritance of secrets or unrestricted host state; environment/cwd/capabilities come from explicit launch data;
- late events from an interrupted or replaced kernel cannot complete a newer request.

### Rust mechanism

Implement in `pi-rs-host` (for example `crates/pi-rs-host/src/python/`) as subprocess supervision, framing, resource limits, and translation to typed effects/events. Use `pi-rs-app` only to wire lifecycle ownership. If a tiny Python helper is required, package it as distribution support, not product policy. All request handling must re-enter Lua through snapshot/action dispatch rather than calling Lua with mutable host state.

### Lua policy

A minimal replaceable Python tool package formats executions and renders results. It decides when to restart after failure and how results enter the transcript; it does not spawn or signal processes directly.

### Go/no-go gate

Use the real Nix-provided Python process, not a mock:

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).python-bridge
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).repl-traces
```

Go only when tests prove persistence across cells, stdout/stderr ordering, Unicode and rich output, exceptions, typed host-request round trips, interrupt and timeout, crash/restart isolation, output limits, shutdown without orphan processes, and deterministic replay of the seed REPL traces. Include adversarial framing and stale-correlation cases.

## Phase 3 — faithful Prime Agent RLM loop in Lua

**Dependencies:** Phase 2.

### Scope

Port the pinned Prime Agent agent loop faithfully before redesigning or generalizing it. Preserve its externally observable turn protocol, prompt/context assembly, persistent-Python interaction, model/tool continuation rules, queue semantics, retries, prose-stop behavior, usage accounting, error presentation, compaction trigger/resume behavior, and cancellation. First match behavior; defer cleanup and new configurability unless needed to express existing policy.

Implement the agent as a replaceable Lua root/package alongside the existing `application`, `frontend`, and `middleware` roots, using `roots.v1.dispatch`. Model inference continues through `pi-rs-ai`; provider selection/auth stays existing Lua product policy. Python work uses only the Phase 2 action/event API.

### Rust mechanism

No agent decisions. Add only missing generic typed actions/effects, bounded queues, or snapshot fields demonstrated by a failing parity trace. Do not create a Rust “agent” state machine. Keep `pi-rs-ai`, `pi-rs-ai-auth`, and provider oracle behavior unchanged.

### Lua policy

Own the complete RLM turn state machine, transcript construction, retry/queue/compaction policy, tool result insertion, prose stop, and product defaults. Split decisions from rendering/machinery in the Lua package, but avoid abstraction churn during parity work.

### Go/no-go gate

Run the real Lua loop, real Python subprocess, and scripted model:

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).rlm-parity
nix flake check
```

The trace suite must cover prose-only completion, one and multiple Python cells, persisted Python state, host request, model/tool/provider error, retry exhaustion, cancellation, queued steering input, compaction then continuation, context overflow, usage totals, and malformed model events. Go only when normalized traces match the pinned TypeScript reference for the agreed corpus and the existing provider/auth oracle suite remains green. This is the first end-to-end usable milestone.

## Phase 4 — generic durable records, session restore, and compaction durability

**Dependencies:** Phase 3.

### Scope

Make the vertical slice survive process/session restoration before building the continual harness or daemon. Reuse pi-rs’s generic durable record store. Define versioned namespaces, atomic/optimistic update semantics, listing, deletion/tombstones, and migration behavior for:

- transcripts and compacted context;
- product session metadata and RLM state needed to resume;
- kernel descriptors (never serialize a live process handle; define whether a restored session gets a fresh kernel);
- future harness records and schedules.

Test crash boundaries around record writes. A partially applied turn must resume or roll back according to one explicit rule, never silently duplicate a tool/Python result.

### Rust mechanism

Extend `pi-rs-session` only with generic record/query/transaction/migration mechanisms needed by multiple policies. Filesystem/SQLite choice, locking, durability, and corruption errors are mechanism. Expose effects through `pi-rs-host` snapshots/actions; do not introduce record-kind enums for Prime features in Rust.

### Lua policy

Define record namespaces, schemas, transcript checkpoints, compaction records, resume reconstruction, retention, and user-facing corruption/conflict behavior.

### Go/no-go gate

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).session-resume
nix flake check
```

Go only when a test kills the application after each enumerated turn boundary, restarts it, and obtains the same next normalized RLM trace without duplicated effects; schema migration, concurrent-version conflict, corruption, compaction restore, and bare ephemeral mode are covered. Verify an ephemeral product still runs with persistence disabled.

## Phase 5 — continual harness and package/skill layers

**Dependencies:** Phase 4.

### Scope

Port `/refine` and the persisted prompt, memory, skill, and reusable subagent-spec layers as Lua policy over generic records. Define precedence and scope explicitly: shipped packages, user-installed packages, session-local refinements, and global refinements. Keep static declarations as data. Refine operations are validated CRUD with audit metadata and deterministic prompt projection.

Package discovery/loading must reuse pi-rs’s public Lua package mechanism. Shipped Prime skills live in `pi-rs-builtins` (or the existing file-backed shipped Lua package tree) but use exactly the same loader and capability declarations as third-party packages. Removing builtins must leave a bootable bare core.

### Rust mechanism

Only generic secure file/package lookup and generic durable-record operations if existing APIs are insufficient. Validate paths, sizes, and declared capabilities. Rust must not know what a “memory,” “prompt note,” or `/refine` means.

### Lua policy

Own schemas and commands for create/update/delete/list, prompt injection order, skill discovery and invocation metadata, local/global scope, audit rendering, conflicts, and `/refine` UX.

### Go/no-go gate

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).continual-harness
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).no-builtins
```

Canonical fixtures must cover CRUD, scope/precedence, restart persistence, invalid records, migration, concurrent edits, skill discovery, capability denial, and deterministic prompt composition. The no-builtins derivation must boot, dispatch a minimal external file-backed package, and complete the documented bare action.

## Phase 6 — recursive subagents and agent-to-agent messaging

**Dependencies:** Phases 3–5. Build before daemonization so semantics are stable in one process.

### Scope

Represent a child as another instance of the same Lua agent root, not a Rust-special agent. Define parent/child identities, maximum depth, child budgets, inherited context/harness visibility, cancellation propagation, lifecycle states, result delivery, family-addressability, bounded mailboxes, message ordering, and failure semantics. Recursive depth and resource limits must be explicit snapshot data. No child receives a shortcut to host state.

Start with deterministic cooperative scheduling in one process. Add only mechanism needed to host multiple isolated Lua roots and route typed events. Persist lifecycle/message records using Phase 4 so a later daemon can supervise them without changing product semantics.

### Rust mechanism

In `pi-rs-host`/`pi-rs-app`, provide generic root-instance IDs, isolated execution contexts, watchdog/budget enforcement, cancellation trees, fair bounded scheduling, and typed mailbox delivery. If durable mailbox primitives are necessary, keep them generic in `pi-rs-session`. No Prime prompt, depth policy, or result aggregation in Rust.

### Lua policy

Own spawn admission, depth policy, budget allocation, context selection, child prompt, child completion interpretation, family roster, message/reply behavior, and transcript presentation.

### Go/no-go gate

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).recursive-agents
```

Canonical/scripted fixtures must cover parent→child→result, child Python use, sibling/parent messaging rules, two levels of recursion, depth denial, budget exhaustion, mailbox backpressure, deterministic ordering, child crash, parent cancellation, watchdog termination, persistence/restart, and absence of mutable cross-root handles. A malicious infinite-loop Lua child must not hang its parent or another child.

## Phase 7 — hard mechanism II: daemon supervisor and thin clients

**Dependencies:** Phases 4 and 6. Do not daemonize an unsettled loop or child model.

### Scope

Introduce the daemon only now, preserving the same Lua session and agent policies. The daemon owns the state that must outlive viewers: live agent/root instances, Python kernel subprocesses, queued/in-flight turns, child tree and mailboxes, heartbeat/schedule timers, and attachment cursors. Durable records remain the recovery source after daemon death; clients attach, render events, send commands, and detach.

Define a small framed local wire protocol with an integer protocol version, request/event IDs, authentication/permissions for the local endpoint, attach/replay cursor, capability negotiation, backpressure, graceful shutdown, and clear rejection of incompatible clients. Additive optional features are capability-gated; breaking changes bump the version. Do not make the frontend depend on daemon internals.

### Rust mechanism

Implement supervisor/worker lifecycle, IPC transport, client multiplexing, attach/replay, process reaping, signals, bounded queues, and recovery. Place it in `pi-rs-app` if that keeps the public surface narrow; create `pi-rs-daemon` only if the protocol/supervisor is independently testable and `DESIGN.md` is updated with that boundary. Continue to run each Lua dispatch under a watchdog. The daemon cannot interpret RLM or harness policy.

### Lua policy

Own session create/resume/end, what is recoverable, attachment presentation, offline notification policy, and responses to worker failure. The existing frontend becomes a thin client package; headless control is another client using the same protocol.

### Go/no-go gate

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).daemon-protocol
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).daemon-continuity
```

Tests must kill and restart clients while a scripted turn and child continue, then reattach and observe an ordered, nonduplicated event stream and intact named state. Also cover daemon crash/recovery, stale sockets, two clients, slow-client backpressure, authorization failure, worker/Python crash, graceful shutdown, and new-client/old-daemon plus old-client/new-daemon behavior. Go only when client death cannot kill daemon-owned state and incompatible protocols fail clearly.

## Phase 8 — background operations: heartbeats, schedules, goals, autonomous mode

**Dependencies:** Phase 7; harness records from Phase 5 and recursive agents from Phase 6.

### Scope

Port heartbeats, schedules, persistent goals, and autonomous mode as Lua policies using daemon timers and agent spawning. Establish clock/time-zone, missed-run, overlap, retry, cancellation, idempotency, budget, and restart semantics from the pinned Prime Agent behavior. A scheduled execution is a normal agent/root invocation and uses normal messages, Python, harness, and records—never a privileged path.

### Rust mechanism

Expose generic monotonic/wall-clock timer actions, wakeups, and durable wake registration in the daemon. Rust enforces resource ceilings and delivers timer events; it does not interpret cron, goals, autonomous prompts, or retry policy unless parsing a declarative schedule is the minimal stable mechanism justified in `DESIGN.md`.

### Lua policy

Own schedule parsing/validation policy, missed-run and overlap decisions, heartbeat prompts, goal lifecycle/progress projection, autonomous continuation/stop decisions, notifications, and commands.

### Go/no-go gate

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).background-policy
```

Run canonical fixtures under a fake clock for on-time, missed, overlapping, retrying, canceled, budget-exhausted, and time-zone/DST cases. Include daemon restart between registration and firing, duplicate-wakeup idempotency, autonomous stop, goal completion, and a heartbeat that spawns a normal child and survives client detachment.

## Phase 9 — full frontend, commands, configuration, and canonical experience

**Dependencies:** Phases 3–8.

### Scope

Port the complete user-visible Prime Agent surface only after its engines work headlessly: startup/attach flow, session picker, package/skill commands, `/refine`, messaging/observation, compaction, goals, schedules, autonomous controls, configuration, status, diagnostics, themes, and error/cancellation presentation. Preserve configurable keybindings; defaults are data declarations, never hard-coded checks. Provide machine-readable headless output in addition to TUI rendering.

### Rust mechanism

Only terminal/display/input primitives in `pi-rs-tui` and app/IPC wiring in `pi-rs-app`. Add no product command branching to Rust. Performance-sensitive rendering changes require before/after benchmarks.

### Lua policy

All command tables, keybinding tables, views, themes, navigation, configuration precedence, capability degradation, and user-visible strings. Optional daemon capabilities must degrade locally rather than blocking attachment or startup.

### Go/no-go gate

```sh
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).canonical-experience
nix flake check
```

Canonical fixtures and controlled-terminal snapshots must cover every top-level command and lifecycle, 80×24 and resized terminals, no-color/headless output, configurable keybindings, missing optional capabilities, attach/reconnect, malformed config, and clean success output. A scripted smoke journey must create a session, use Python, refine memory, spawn/message a child, compact, detach, run background work, reattach, and inspect the result.

## Phase 10 — parity closure, performance contracts, and release cut

**Dependencies:** all prior phases.

### Scope

Close the manifest from Phase 0, delete superseded scaffolding, document intentional differences, and establish measured performance/resource contracts. Benchmark mechanisms rather than promising Rust speed abstractly:

- cold CLI and daemon startup;
- Lua dispatch latency and watchdog overhead;
- model-event throughput;
- Python kernel cold start and warm cell latency;
- TUI render latency;
- memory per idle session/kernel/child;
- daemon attach/replay and mailbox throughput;
- durable write/restore latency.

Set thresholds from repeated measurements on a named Nix environment. Optimize only failures, preserving trace parity. Audit public module surfaces and confirm shipped product remains removable.

### Rust mechanism

Instrumentation, bounded diagnostic dumps, benchmark harnesses, leak/process-reaping checks, packaging, and only evidence-driven optimizations. Retain protocol/schema compatibility tests and provider/auth corpus as permanent release checks.

### Lua policy

Finish fixture parity, diagnostics/inspect commands, documentation, default package manifests, and removal of temporary compatibility glue. Any conscious divergence is recorded in `DESIGN.md` with rationale, not hidden by fixture normalization.

### Go/no-go gate

```sh
nix flake check
nix build .#prime-agent
nix run .#prime-agent -- --machine-readable smoke
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).bench-contracts
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).no-builtins
```

Release only when:

- provider/auth oracle, scripted-LM+REPL-trace, and canonical-experience suites all pass;
- every Phase 0 manifest row is implemented, explicitly deferred, or documented as an intentional difference;
- performance/resource thresholds and process-leak checks pass;
- killing/restarting a client preserves daemon-owned state, and killing/restarting the daemon recovers all promised durable state;
- the bare core boots and performs the `DESIGN.md` bare action without builtins;
- a file-backed external package can replace each application/agent/frontend/middleware root through the public API;
- no Rust module contains Prime-specific policy that could have been expressed by the Lua snapshot/action interface.

## Dependency spine and scope control

The critical path is:

```text
contract freeze
  → typed effect boundary
  → Python/IPython mechanism
  → faithful RLM Lua loop
  → durable restore
  → continual harness
  → recursive agents/messages
  → daemon mechanism
  → background policies
  → full experience and release closure
```

Parallelism should be limited to work with stable inputs. After Phase 1, reference-trace capture can continue alongside the Python bridge. After Phase 4, harness fixture capture may overlap late recursive-agent work. Frontend fixture capture can run ahead, but frontend implementation should not define daemon or agent semantics.

At every phase, reject these tempting expansions:

- rewriting agent decisions in Rust for performance;
- a Prime-only Python, record, timer, child, or messaging API;
- daemonizing before in-process lifecycle semantics pass;
- redesigning the RLM loop while parity failures remain;
- broad plugin frameworks beyond the existing public roots/package system;
- compatibility promises beyond provider/auth exhaustiveness and the explicitly versioned Prime protocols/schemas;
- optimization without a failing benchmark contract.
