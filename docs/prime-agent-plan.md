# Reconcile note (applied before this plan was committed)

The synthesized plan below originally resolved repo shape as **fork pi-rs ->
prime-rs**. That decision is **reversed** here per follow-up design work:

- **Best of both worlds:** keep pi-rs as the base, work on the `prime-agent`
  **branch**, keep the generic mechanism crates (`pi-rs-repl`,
  `pi-rs-daemon`) under the kernel namespace and upstreamable, and ship the
  whole product as a **Lua package overlay** (no privileged builtins,
  removable by ablation). This is the "Fable vs Kimi/Sol" fork-vs-branch
  disagreement re-resolved in favour of branch, because pi-rs is
  Lua-configured by design — a separate repo buys nothing a branch +
  package-overlay does not.

The phase plan, mechanism crates, parity strategy, sequencing, and gates are
all unchanged and adopted as written. Only the repo-shape headline (decision #1
and synthesis note #1) differs: substitute "branch + Lua package overlay on
pi-rs" wherever "fork pi-rs -> prime-rs" appears. DESIGN.md (the section
"Prime Agent on pi-rs — product pending") records the locked decisions D-P1..D-P6.

---

# Prime Agent on pi-rs — Final Reconciled Build Plan

Synthesized from three independent plans (GPT 5.6 Sol, Claude Fable 5, Kimi K3).
Where they agree, the point is stated as settled. Where they differ, a winner is
named with a one-line reason. Disagreement resolutions are listed at the end.

## Settled by unanimous agreement (do not reopen)

- **Python/IPython bridge before the RLM loop; RLM loop before the daemon; daemon
  before subagents-at-scale, heartbeats, and autonomy.** All three plans converge
  on this spine.
- **The RLM loop is ported faithfully as Lua policy** — a replaceable agent root,
  no redesign until parity is green, no Rust agent state machine ever.
- **Parity strategy per layer is fixed:** provider/auth = existing oracle fixtures
  (untouched, permanently green); Python bridge + RLM loop = scripted-LM +
  REPL-trace over a **real** Python subprocess (phi's proven shape, 82-test seed
  corpus of scenarios); harness/daemon/subagents/experience = canonical fixtures.
- **The REPL wire protocol carries `host_request` mid-execution from day one** —
  model-written Python awaiting a typed agent capability is the subtle part and
  cannot be retrofitted.
- **Versioned protocols everywhere:** one integer version on the REPL framing and
  on the daemon wire, bumped on breaking change; additive changes keep old
  clients; incompatible clients get a clear rejection, never a misparse.
- **No privileged path:** every shipped Prime package loads through the same
  public loader and root/command declarations a third-party package uses; a
  bare-core/ablation CI job exists from Phase 0 and is a release blocker.
- **Nix is the only verification sentence.** A phase gate is a quoted
  `nix flake check` / `nix build` / `nix run` exiting zero.
- **Skills need no second tool runtime** (Fable + Kimi, Sol compatible): the REPL
  is the model's tool; Python-backed skills are preloaded/registered into the
  kernel via the existing `execute` path; markdown skills are context data.

## Headline decisions (conflicts resolved)

1. **Repo shape: fork pi-rs → `prime-rs` at a pinned commit** (Fable), mirroring
   how Prime Agent forked pi. Keeps the user's pi-rs clean and its oracle corpus
   pinned; the fork records its base commit in DESIGN.md.
2. **Two new mechanism crates, named `pi-rs-repl` and `pi-rs-daemon`** (Kimi's
   names, Fable's count). Rationale: both crates are Prime-agnostic mechanism —
   Rust must not know what a memory, heartbeat, or RLM turn is — so they carry
   the kernel's namespace and remain upstreamable to pi-rs. No other new crates;
   Sol's "maybe fold into pi-rs-host/pi-rs-app" loses because both pieces are
   independently testable with coherent public boundaries (the pi-rs 8-crate
   granularity precedent).
3. **Crates are created at their phase, not as Phase-0 skeletons** (Fable/Sol
   over Kimi): least-code; an empty crate is scaffolding nobody exercises.
4. **Ordering: REPL → record-store CRUD → RLM loop → harness → daemon →
   subagents → autonomy → release** (Kimi's spine). Subagents land **after** the
   daemon (Fable + Kimi over Sol): Prime Agent's messaging and child lifetimes
   are daemon-backed in reality, and the daemon's one-dispatch-engine proof
   (Phase 5 gate) covers Sol's "settle semantics in one process first" concern
   without a throwaway in-process scheduler.
5. **Shipped Prime product lives under `crates/pi-rs-builtins/<area>/`** (rlm/,
   harness/, daemon/, subagents/, autonomy/) added to `default.json` in load
   order (Kimi over Fable's `lua/prime/` tree): it reuses the existing shipped-
   package distribution mechanism verbatim, and the zero-pack/ablation gates
   prove removability regardless of directory.
6. **Scripted-LM seam = the public `register_api_provider` path** in
   `crates/pi-rs-ai/src/registry/api_registry.rs` (Fable + Kimi, explicit): a
   test provider replaying a fixture script of `text_delta`/`toolCall` events
   through the same registration real providers use. No dedicated test hook.
7. **RLM depth control = Lua policy counter, kernel bound as backstop.** The
   product depth limit (Prime Agent's configurable RLM depth) is explicit
   snapshot data owned by the spawn-admission policy in Lua; the kernel's
   existing `MAX_NEST_DEPTH = 8` in `crates/pi-rs-host/src/kernel.rs` remains
   the hard mechanism ceiling. No new mechanism (reconciles Kimi's reuse with
   Sol's "depth must be explicit snapshot data").
8. **Record-store delta is a small, early, parallelizable phase** (Kimi over
   Sol's post-loop placement): a named-record CRUD layer (put/get/delete/list,
   tombstone appends, latest-value fold) on the existing append-only
   `pi-rs-session` store. Harness and daemon both consume it; building it early
   unblocks both.
9. **DESIGN.md before code, in the fork** (all three): one DESIGN.md with the
   four canonical sections, recording Kimi's locked decisions D-P1..D-P4
   (REPL is mechanism; REPL is scope-owned, never global; daemon activates the
   deferred doctrine-03 row with a named owned-state set; RLM loop is a
   replaceable Lua agent root), plus Fable's explicit sentence: "the RLM loop is
   ported faithfully from Prime Agent TS; redesign is out of scope until parity
   is green."

## Reference pins (Phase 0 deliverables)

- pi-rs base commit (the fork point).
- Prime Agent (TypeScript) reference commit — the behavioral spec for the value
  layer.
- Pi v0.79.0 provider/auth oracle corpus — inherited, never modified.
- A seed fixture manifest (Sol's list, adopted verbatim): prose-only stop; one
  Python execution + model continuation; Python state across two executions;
  Python exception and timeout; one compaction boundary; one `/refine`
  mutation; one child/message lifecycle; daemon detach/reattach; heartbeat,
  schedule, goal, and autonomous-mode examples. Each row ends the project as an
  implemented fixture, an explicit deferral, or a documented intentional
  difference.

---

## Phase 0 — fork, DESIGN.md, CI baseline

**Scope.** No features. Fork pi-rs → `prime-rs` at a pinned commit. Write
DESIGN.md (four sections) with the locked decisions above, the mechanism/policy
placement table for every Prime feature, the daemon-owned state named as a
sentence ("live session reducers + their record-store writers + the job
scheduler"), the extension-boundary module named for the functional-core grep
check, and the doctrine-03 row flipped from deferred to follows (citing D-P3).
Record the seed fixture manifest. Wire CI: inherited pi-rs suite plus the
bare-core/no-privileged-path job (build with Prime packages and builtins
removed; confirm the bare core boots and performs the documented bare action).

**Rust / Lua.** Neither. Docs + flake + CI only.

**Gate.**
```sh
rg '^## (Locked decisions|Architecture|Deferred|Roadmap)' DESIGN.md   # exactly 4 hits
nix flake check                                                        # inherited suite + oracle fixtures green
```
Plus the bare-core CI job green.

---

## Phase 1 — `pi-rs-repl`: persistent Python/IPython bridge (hard mechanism #1)

**Scope (Rust mechanism, new crate `pi-rs-repl`).**

- One long-lived Python child per agent scope, running a small vendored shim
  (`prime_repl_shim.py`, shipped as crate data) embedding
  `IPython.core.interactiveshell.InteractiveShell`. **Shim + framed JSON-lines
  over stdio, not Jupyter/ZMQ** (Fable + Kimi over Sol's "prefer Jupyter":
  ipykernel drags in a message spec, heartbeat channels, and HMAC the product
  does not need — least-code; "IPython compatibility" is an executable trace
  contract, not a wire-protocol adoption). Python environment pinned in Nix.
- Framed, length-prefixed JSON protocol, one integer version. Frames:
  host→shim `execute{id, code}`, `interrupt{id}`, `host_response{req_id,
  result}`, `snapshot{}`, `shutdown{}`; shim→host `stream{id, name, chunk}`
  (bounded chunks, never per-byte), `result{id, ok, value, error}`,
  `host_request{req_id, kind, payload}` (mid-execution, from day one),
  `snapshot_data{...}` (best-effort pickle of revivable names for session
  revival). Correlation IDs; late events from an interrupted/replaced kernel
  cannot complete a newer request (Sol's stale-correlation rule).
- Reuse pi-rs effect substrate: bounded channels, `EffectOptions::long_lived()`
  for the channel, a **per-cell watchdog** for each execute — SIGINT to the
  child's process group, escalate to SIGKILL + respawn; kernel death is a typed
  event, never a hang. Scope disposal kills the child (process-group reap).
- Byte-capped stream buffers with typed truncation markers (bounded tool
  output is mechanism-level).
- No secret/host-state inheritance: env, cwd, capabilities come from explicit
  launch data.
- Lua seam `pi.repl.v1` (`start(opts) -> handle`; `execute`, `interrupt`,
  `complete`, `shutdown`), installed from `pi-rs-host/src/bindings/` beside
  `effects.rs`. Lua never holds a process handle; a file-backed package could
  provide the same seam.

**Scope (Lua policy).** One throwaway smoke tool for the gate only. The real
`python` tool ships with the loop in Phase 3.

**Parity.** REPL-trace: `crates/pi-rs-repl/tests/` drives a real Python
subprocess through recorded cell scripts and asserts event traces —
persistence across cells, stdout/stderr ordering, Unicode/rich output,
exceptions, `%%bash`-style cells if ported, interrupt of `while True: pass`,
watchdog kill + respawn, mid-execution `host_request` round trip,
snapshot/revive, adversarial framing, stale correlation.

**Gate.**
```sh
nix build .#pi-rs-repl
nix flake check          # REPL trace corpus green against real Python
nix run .#repl-smoke     # `x = 1` then `x + 1` -> 2 across two frames
```
Plus: kill -9 mid-execute → typed death event within the watchdog budget, next
execute works on a fresh kernel; N start/dispose cycles leave zero surviving
python children and stable RSS.

---

## Phase 2 — named-record CRUD layer in `pi-rs-session` (small; parallel with Phase 1)

**Scope (Rust mechanism).** Extend
`crates/pi-rs-session/src/record_store.rs` (or a sibling module) with a
named-record layer over the append-only log: records `{collection, key,
op = put|delete, value}` folded to a latest-value view; deletes are tombstone
appends; `fold_collection(name)` streams within existing `StoreLimits`. The
store keeps opaque JSON — collection/key meaning is data, not new store verbs.
Expose additively via `pi.records.v1`: `put/get/delete/list`. Atomic append,
locking, and loud corruption reporting preserved. Store compaction deferred
until a measured problem exists.

**Scope (Lua policy).** None (harness schema is Phase 4).

**Parity.** Contract tests in `crates/pi-rs-session/tests/`, not oracle.

**Gate.**
```sh
nix flake check
```
Green on: put/get/delete round trip; latest-value-wins; tombstone hides key on
list; fold respects window limits; corruption reported, not swallowed;
kill-during-append leaves the store readable.

---

## Phase 3 — RLM agent loop, ported faithfully (Lua policy; the value)

**Scope (Lua policy) — `crates/pi-rs-builtins/rlm/`, in `default.json` after
the `agent/*` entries.**

- **Faithfulness rule (Fable, adopted verbatim):** read the TS loop in full
  first; port control flow 1:1. Divergences forced by the snapshot/action shape
  are listed in DESIGN.md with reasons. No improvements until parity is green.
- `rlm/loop.lua` (`pi.rlm.loop@1`): agent root registered at priority > 0
  through the public `roots.register` seam, replacing `pi.builtins.agent`
  (priority 0) without privilege. Reuses `pi.agent.queue@1`
  (steer/follow-up/interrupt) and `pi.agent.tools@1` unchanged. Owns the turn
  protocol, tool loop, retry, prose-stop detection, usage accounting, error
  presentation, cancellation, and declared limits (`max_requests`,
  `max_tool_iterations`, plus RLM bounds).
- `rlm/compact.lua`: compaction trigger + summarization policy as a Lua reducer
  emitting the session package's existing compaction-record concept.
- `rlm/tools/python.lua`: the shipped `python` tool over `pi.repl.v1`, declared
  via `pi.agent.tools@1` with `serialize = true` (one kernel, no interleaving)
  and `owner = "pi.rlm"`. Bounded output via the Phase 1 caps.
- Context assembly: system prompt as data tables/templates (`rlm/prompts/`),
  project AGENTS.md discovery, harness block slot (filled in Phase 4).
- Depth plumbing: current RLM depth as snapshot data in the system prompt; the
  limit is policy config; spawning arrives in Phase 6.

**Scope (Rust mechanism).** Ideally none. A missing primitive (timer,
cancellation seam, read-only depth accessor) is added generically to
`pi-rs-host`, only with a failing parity trace as evidence. Never an
RLM-specific hook.

**Parity.** Scripted-LM + REPL-trace, the headline suite: a scripted provider
registered through public `register_api_provider`
(`crates/pi-rs-ai/src/registry/api_registry.rs`) replays fixture scripts; the
real loop root dispatches over a real `pi-rs-repl` kernel; assertions on the
full normalized action trace (turn start, tool start/result, message,
prose-stop, compaction, stop reason) and REPL side effects. Seed corpus =
phi's 82 scenario shapes: multi-turn tool loops, error turns, retry
exhaustion, prose-stop, interrupt mid-tool, queued steering, compaction at
threshold, context overflow, malformed model events, context assembly. Where
the TS Prime Agent can be driven deterministically, capture its transcripts
and diff shapes (canonicalize timestamps/IDs; never normalize ordering, stop
reasons, or errors).

**Gate.**
```sh
nix flake check    # scripted-LM+REPL-trace suite >= phi scenario coverage
nix build          # with the rlm package removed: bare agent still boots (ablation)
```
Plus one documented manual smoke: `nix run .#prime -- -p "compute 2**10 in
python"` against a real provider completes a full turn through the real kernel.
This is the first end-to-end usable milestone.

---

## Phase 4 — continual harness: `/refine`, memories, skills, prompts (Lua over the store)

**Scope (Lua policy) — `crates/pi-rs-builtins/harness/`.**

- `harness/store.lua` (`pi.harness.store@1`): schema over Phase 2 collections —
  `memories`, `skills`, `prompts`, `subagents`, `refinements`; entries carry
  `scope = "local"|"global"` and audit metadata. Local vs global = two store
  paths chosen by Lua path policy (`pi.config.paths@1` precedent). Rust knows
  none of these names.
- `harness/refine.lua` (`pi.harness.refine@1`): `/refine` as an ordinary
  command declaration (`pi.roots.v1.declare("command", ...)`). Validated CRUD +
  rollback + `record_refinement`.
- `harness/assemble.lua`: deterministic prompt projection from local+global
  entries with local-over-global precedence — a pure function, trivially
  fixture-testable. Consumed by the RLM loop's turn start.
- Skills: markdown skills surface into context; Python-backed skills preload
  into the kernel through Phase 1's execute path. No new frames, no second
  runtime.
- Harness state also survives restore: transcript checkpoints and resume
  reconstruction policy land here (Sol's durability concern), with the
  crash-boundary exactly-once rule stated in DESIGN.md ("a partially applied
  turn resumes or rolls back by one explicit rule, never duplicates a
  tool/Python result") and tested at the Phase 5 gate.

**Scope (Rust mechanism).** None beyond Phase 2.

**Parity.** Canonical fixtures: harness state in → rendered prompt block out;
CRUD round trips surviving process restart; scripted-LM scenario where the
model performs a harness op mid-turn; scope/precedence, invalid records,
concurrent-edit conflict.

**Gate.**
```sh
nix flake check
nix build          # harness package removed: boots, RLM loop runs with empty harness
```
Fixture: fresh session → create memory/skill/prompt note → restart → all three
appear in the injected overview.

---

## Phase 5 — `pi-rs-daemon`: supervisor + versioned wire (hard mechanism #2)

**Scope (Rust mechanism, new crate `pi-rs-daemon`).** Sequenced here, after
the loop and harness, because they define what state the daemon owns; before
subagents and autonomy, which stack on it.

- Supervisor owns the named state: live session reducers (each worker = a
  headless `pi-rs-app` instance running the same Lua package graph — **one
  dispatch engine, not two**), single writer per durable record store, the job
  scheduler (timers firing scheduled dispatches — built here, consumed in
  Phase 7), Python kernel subprocess lifetimes, and attachment cursors.
  Durable records are the recovery source after daemon death.
- Wire: Unix socket at `$XDG_STATE_HOME/pi/daemon.sock`, length-prefixed JSON
  (same framing idiom as the REPL — one implementation), one integer
  `DAEMON_WIRE_VERSION`. Verbs (mechanism-only vocabulary): `attach`, `detach`,
  `dispatch`, `subscribe`, `spawn_background`, `list_sessions`, `schedule`,
  `unschedule`, `list_jobs`. Capability negotiation for additive features;
  local endpoint auth; backpressure on slow clients; graceful shutdown;
  worker death is a supervisor event + session-record state, never supervisor
  death; watchdog on worker handshake and every dispatch.
- Thin-client mode in `pi-rs-app/src/launcher.rs`: the existing frontend
  renders frames from the socket; attach/detach are pure viewer operations.
  Single-process mode from Phases 1–4 keeps working (daemonized operation is
  optional policy).

**Scope (Lua policy).** `crates/pi-rs-builtins/daemon/`: when a session goes
background, naming, reattach presentation, what a resumed session revives
(including kernel `snapshot_data` revival), worker-failure user messaging.

**Parity.** Canonical continuity journey + contract tests in
`crates/pi-rs-daemon/tests/`: protocol frame fixtures per verb;
version-mismatch rejection; new-client/old-daemon and old-client/new-daemon;
two clients; slow-client backpressure; stale socket; soak (10 workers, bounded
RSS, counted fds).

**Gate.**
```sh
nix run .#prime-daemon &
nix run .#prime -- attach <id>
nix flake check
```
Green on: the doctrine-03 run-check (start turn, kill -9 the client, reattach,
named state intact and the in-flight turn completed); daemon crash → restart →
promised durable state recovered without duplicated effects (the exactly-once
crash-boundary rule from Phase 4, tested at each enumerated turn boundary);
wire-version rejection fixture; **one-dispatch-engine proof**: the Phase 3
scripted trace produces an identical action batch in-process and through the
socket.

---

## Phase 6 — recursive subagents + agent-to-agent messaging (Lua over daemon/scopes/store)

**Scope (Lua policy) — `crates/pi-rs-builtins/subagents/`.**

- `subagents/spawn.lua`: a child is another instance of the same Lua agent
  root in a new scope via `spawn_background` — never a Rust-special agent, no
  new `DeclarationKind`. Admission returns a handle (id, name, depth)
  immediately; results arrive only via messaging or files, never as a spawn
  return value. Spawn admission owns the **policy depth counter** (explicit
  snapshot data, configurable), refusing past the product limit; the kernel's
  `MAX_NEST_DEPTH = 8` remains the mechanism backstop. Child budgets,
  inherited context/harness visibility, and cancellation propagation are
  admission-time data.
- `subagents/message.lua`: routing restricted to parent/siblings/direct
  children as a **table of reachability rules** (least-power, rung 2; tested
  as a pure function over family-tree fixtures). A message is a durable record
  addressed by agent id, delivered as an event on the target's next dispatch —
  data in the store + existing event dispatch, not a new bus. Sender identity
  = sending scope's provenance (no spoofing).
- `subagents/observe.lua`: read-only family status + bounded recent-message
  previews from the same store.
- `agent_message`/`agent_observe` as Python skill modules preloaded into the
  kernel (Phase 4 mechanism) issuing `host_request` frames (Phase 1) routed by
  Lua policy to daemon verbs (Phase 5). This phase is mostly composition — the
  point of the sequencing.

**Scope (Rust mechanism).** Minimal: parent linkage recorded in the session
record, message routing in the supervisor — generic daemon capabilities. A
cross-scope signal, if needed, is a generic scoped event in `pi-rs-host`, with
evidence.

**Parity.** Scripted-LM for behavior (two scripted providers: parent spawns
child, child replies via `agent_message`, parent transcript shows delivery —
over the real daemon + real kernels); canonical for mailbox contents;
depth-cap and reachability-violation fixtures.

**Gate.**
```sh
nix flake check
```
Green on: parent→child→message round trip; spawn at the depth limit → typed
refusal, not a hang; out-of-family message rejected; disposing the parent
cancels the child's in-flight work while its durable messages persist;
mailbox backpressure; a malicious infinite-loop child cannot hang its parent
(watchdog). One documented manual smoke: real model, `await rlm('say hi and
reply')`.

---

## Phase 7 — heartbeats, schedules, goals, autonomous mode (Lua over the daemon scheduler)

**Scope (Lua policy) — `crates/pi-rs-builtins/autonomy/`.**

- `autonomy/heartbeat.lua`: scheduled jobs dispatching wake events into agent
  roots via the daemon `schedule` verb; start/stop/list as command
  declarations; heartbeat prompts as data.
- `autonomy/goals.lua`: persistent goals with budget tracking in the harness
  store; status injected via `harness/assemble.lua`.
- `autonomy/autonomous.lua`: bounded self-continuation toward a goal, budget
  counters checked in the loop, watchdog-bounded, surfaced as command + status.
- Semantics pinned from the TS reference: missed-run, overlap, retry,
  idempotency on duplicate wakeups, cancellation, time-zone/DST. A scheduled
  execution is a normal agent invocation — never a privileged path.

**Scope (Rust mechanism).** None new — the scheduler (durable wake
registration, fake-clock seam for tests) shipped with Phase 5. Rust delivers
timer events and enforces ceilings; it does not interpret cron, goals, or
autonomy.

**Parity.** Canonical schedule/goal state fixtures under a fake clock
(on-time, missed, overlapping, retrying, canceled, DST); scripted-LM autonomy
scenario (3-turn budget → typed budget-exhausted stop); daemon-restart-between-
registration-and-firing fixture.

**Gate.**
```sh
nix flake check
```
Plus: schedules and goals survive daemon restart and client detach/reattach; a
heartbeat fires into a detached session and the transcript shows the turn on
reattach (manual smoke, documented).

---

## Phase 8 — experience sweep, packaging, performance contracts, release

**Scope.**

- Full user-visible surface polish as Lua data: command tables, configurable
  keybinding tables (defaults are data declarations), themes, status,
  diagnostics, machine-readable headless output alongside TUI rendering.
  Optional daemon capabilities degrade locally, never blocking attach/startup.
- Canonical-experience release suite: a scripted end-to-end journey — create
  session, run Python, refine a memory, spawn/message a child, compact,
  detach, heartbeat fires, reattach, inspect — recorded as the regression set;
  80×24 + resize + no-color terminal snapshots.
- Skill/package distribution over `pi.packages.v1` with normal
  provenance/conflict rules.
- Performance by contract: extend `tests/performance/` — REPL cell round trip,
  scripted RLM turn latency, daemon attach/dispatch round trip, harness fold,
  kernel cold start, memory per idle session/kernel/child. Budgets recorded in
  DESIGN.md **only after** a measured release run exists; optimize only
  failing budgets, preserving trace parity.
- Close the Phase 0 manifest: every row implemented, explicitly deferred, or
  documented as an intentional difference — the one judge-check reviewed by
  the user.
- Re-run the full matrices with Prime packages present and removed: zero-pack,
  per-package ablation, whole-root replacement, stale-handle, watchdog,
  cancellation, XDG.

**Gate.**
```sh
nix flake check                          # all suites: oracle, scripted-LM+REPL-trace, canonical, budgets
nix build .#prime && nix run .#prime -- --machine-readable smoke
nix run github:y0usaf/prime-rs           # cold-run demo works
```
Release blockers: bare-core job green; provider/auth oracle untouched-green;
kill/restart client preserves daemon-owned state and kill/restart daemon
recovers all promised durable state; a file-backed external package can
replace each root through the public API; no Rust module contains
Prime-specific policy expressible via the snapshot/action interface.

---

## Dependency spine

```
P0 fork + DESIGN + CI
 ├─► P1 pi-rs-repl (Rust, hard #1) ──────┐
 └─► P2 record CRUD (Rust, small) ───┐   │   (P1 ∥ P2)
                                     │   │
        P3 RLM loop (Lua, faithful) ◄┴───┘
             └─► P4 continual harness (Lua over store)
                  └─► P5 pi-rs-daemon (Rust, hard #2; owns state defined by P3/P4)
                       ├─► P6 subagents + messaging (composition of P1+P4+P5)
                       └─► P7 heartbeats/goals/autonomy (daemon scheduler)
                            └─► P8 experience + performance + release
```

## Standing checks (every phase)

- `nix flake check` is the only sentence meaning "tests pass".
- Bare-core/no-privileged-path job on every commit from Phase 0.
- Provider/auth oracle fixtures never modified; a red oracle halts the phase.
- Functional-core grep on the DESIGN.md-named boundary module (zero
  `&mut`-host-state hits) as a CI text check.
- Every new dispatch/effect gets a watchdog at introduction.
- Reject at every phase (Sol's list, adopted): Rust rewrites of agent
  decisions; Prime-only Rust APIs for Python/records/timers/children/messages;
  daemonizing unsettled semantics; redesigning the loop while parity fails;
  plugin frameworks beyond the existing roots/packages; optimization without a
  failing benchmark.

---

## Synthesis notes — top disagreements and resolutions

1. **Repo shape.** Fable: fork `prime-rs`; Kimi: build in pi-rs; Sol: silent
   (assumes pi-rs). **Fork wins** — mirrors the pi → Prime Agent lineage,
   keeps pi-rs and its oracle pin clean; generic mechanism crates remain
   upstreamable.
2. **New-crate names/count.** Fable: `prime-repl`/`prime-daemon`; Kimi:
   `pi-rs-repl`/`pi-rs-daemon`; Sol: fold into `pi-rs-host`, maybe
   `pi-rs-daemon` later. **Two crates, `pi-rs-*` names** — the mechanism is
   Prime-agnostic by doctrine, so it carries the kernel namespace; separate
   crates win over Sol's folding because both are independently testable
   boundaries. Crates created at their phase, not as empty Phase-0 skeletons.
3. **Subagents before or after the daemon.** Sol: in-process first (Phase 6
   before daemon Phase 7); Fable/Kimi: after. **After wins** (2:1, and Prime
   Agent's messaging is daemon-backed); Sol's semantics-stability concern is
   absorbed by the Phase 5 one-dispatch-engine proof, avoiding a throwaway
   in-process scheduler.
4. **Record-store CRUD timing.** Kimi: small early phase before the loop;
   Sol: after the loop; Fable: implicit in harness phase. **Early wins** —
   it is small, parallel with the REPL, and both harness and daemon consume it.
5. **REPL protocol substrate.** Sol: prefer Jupyter kernel protocol; Fable/
   Kimi: vendored shim + framed JSON-lines. **Shim wins** (least-code; the
   IPython contract is enforced by executable traces, not wire adoption).
6. **Scripted-LM seam.** All effectively converge; made explicit: the public
   `register_api_provider` registry path, no dedicated test hook.
7. **RLM depth control.** Kimi: reuse kernel `MAX_NEST_DEPTH`; Sol: explicit
   snapshot data + policy. **Both**: Lua policy counter as configurable
   snapshot data (product behavior), kernel bound as the hard mechanism
   backstop.
8. **Lua product placement.** Fable: `lua/prime/` tree; Kimi:
   `crates/pi-rs-builtins/<area>/` + `default.json`. **Builtins tree wins** —
   it is the existing shipped-package mechanism; removability is proven by the
   ablation gates, not by directory layout.
9. **Frontend/experience as its own phase.** Sol: dedicated Phase 9;
   Fable/Kimi: folded into the sweep. **Folded wins**, but Sol's canonical
   smoke journey and terminal-snapshot grid are adopted into Phase 8.
10. **Turn-boundary durability.** Sol's dedicated crash-boundary/resume phase
    is not kept as a phase, but its exactly-once rule and kill-at-each-turn-
    boundary test are adopted: the rule is stated in Phase 4 and gated in
    Phase 5.
