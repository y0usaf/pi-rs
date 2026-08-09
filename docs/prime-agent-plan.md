# Prime Agent on the faithful pi-rs port — build plan

This document supersedes the old Prime Agent build plan. The old plan assumed
the bounded-parity base; it is archived, unmodified, on branches `prime-agent`
(`docs/prime-agent-plan.md`) and `main-bounded-parity`. The base is now the
faithful port: `DESIGN.md` is the product contract, `PLAN.md` is the parity
roadmap, and `LUA_SURFACE.md` defines the three-tier public Lua surface. This
plan changes none of them. Where this document and `DESIGN.md` disagree,
`DESIGN.md` wins.

## Sequencing

Two tracks, in parallel, one standing rule: **a red parity check halts Prime
work.** The parity suites (`final-parity-audit`, differential frames,
`bare-boot`, `extension-parity`, `dogfood-fixtures`) run on every Prime
commit; the default Pi-parity composition and its suites stay green untouched.

1. **Parity track (`main`).** Complete `PLAN.md` — A.1–A.3, 9.2–9.11, 8, 10,
   11 — until the final parity and ablation audit is green and the parity
   baseline is tagged. This track owns the release gate: the parity tag waits
   for it, but Prime's start no longer does.
2. **Prime track (`prime-agent-faithful` branch).** Port the Prime Agent
   product from its TypeScript reference: the RLM loop, the continual harness
   (`/refine`, memories, skills, prompts), recursive subagents with
   agent-to-agent messaging, the persistent Python/IPython tool, daemon-backed
   continuity, heartbeats/schedules/goals, and autonomy. The engine is Rust;
   the product is Lua policy over the faithful port's public mechanism seams.
   "Build it in Rust" means build it on the Rust engine — repo `pi-rs`,
   product policy in Lua. Prime is **additive**: a separate composition of
   ordinary Lua packages loaded through the public loader. It never joins the
   default builtins manifest. The branch tracks `main` continuously; Prime
   merges never carry parity work backward.
3. **Dependency gating inside the Prime track.** P1 (`pi-rs-repl`) and P2
   (named-record CRUD) are additive, isolated crates/layers and start
   immediately. P3 (RLM loop) lands only once the agent-policy replacement
   seam (PLAN 9.10) exists on `main` — the loop replaces `agent-policy`
   through the public registration seam, and that seam is not built yet.
4. **Lua-surface extension fourth.** Grow the three-tier public Lua surface
   (`LUA_SURFACE.md`) reactively, discovered by building the products — Prime
   first, then a second non-Prime harness as the any-harness proof. No
   spec-first surface growth: a tier-2 mechanism lands only when a real
   consumer already needs it.

## Relationship to the parity contract

- `DESIGN.md`'s exhaustive difference list gains no row for Prime. The parity
  product remains the default composition; Prime adds no pi-rs-specific
  behavior to it.
- The parity suites (`final-parity-audit`, differential frames, `bare-boot`,
  `extension-parity`, `dogfood-fixtures`) run on every Prime commit. A red
  parity check halts Prime work.
- Prime packages load only through the public package/config loader — the
  same path a third-party package uses. No embedded-pack privilege, no
  source-name checks, no Prime-only Rust hooks. This is the no-privileged-path
  proof in the two-composition world: Prime is file-backed by construction.
- Where building Prime exposes a missing host capability, it is added as a
  generic tier-2 additive mechanism with a file-backed exerciser
  (`LUA_SURFACE.md` tier 2 rules), never as Prime-specific policy in Rust.
- Rust is mechanism only. Rust must not know what a memory, a heartbeat, an
  RLM turn, a goal, or an agent message is.

## Reference pins

Recorded at P0; the behavioral spec for each layer.

| Pin | Role |
|---|---|
| `DESIGN.md` / `PLAN.md` / `LUA_SURFACE.md` on `main` | The faithful-port contract; P0 closes `PLAN.md` |
| Prime Agent (TypeScript) reference commit | The value-layer behavioral spec — pinned as a hash-locked Nix oracle input, the same treatment `PLAN.md` A.3 gives Pi; never an ambient sibling checkout in CI |
| Pi v0.79.0 `ref/pi` @ `c5582102` | The product parity spec, inherited, untouched |
| phi's 82-scenario corpus, preserved pre-port tree (`fd373e0`) | Seed scenarios for the scripted-LM + REPL-trace suite |

## Mechanism/policy placement

Every Prime feature is one of: a new **generic** Rust mechanism crate
(Prime-agnostic, upstreamable to the port, created at its phase — never as an
empty skeleton), an additive mechanism row in an existing crate, or pure Lua
policy over public seams.

| Feature | Rust mechanism | Lua policy |
|---|---|---|
| Persistent Python/IPython tool | New crate `pi-rs-repl` (P1): one long-lived Python child per agent scope running a vendored IPython shim; framed, length-prefixed JSON-lines over stdio (not Jupyter/ZMQ) with one integer version; per-cell watchdog (SIGINT to the child process group, escalate SIGKILL + respawn); byte-capped stream buffers with typed truncation; `host_request` mid-execution frames from day one; stale-correlation rejection; scope-owned child lifetime. Exposed to Lua as `pi.repl`, a tier-2 binding in `pi-rs-host` backed by the crate. | The shipped `python` tool declaration in the RLM package; output-budget policy; Python-backed skills preloaded into the kernel through the same execute path (no second tool runtime). |
| Named-record storage | Additive record layer in `pi-rs-session` (P2): `{collection, key, op = put\|delete, value}` appends over the existing append-only log; tombstone deletes; latest-value fold within existing store limits; opaque JSON — collection names are data, not store verbs. Lua seam `pi.records`. | Harness schema (`memories`, `skills`, `prompts`, `subagents`, `refinements`); local/global scope as Lua path policy; deterministic prompt projection. |
| RLM agent loop | None new. A missing primitive (timer, cancellation seam, read-only accessor) lands generically in `pi-rs-host` only with a failing trace as evidence — never an RLM-specific hook. | The entire loop: agent root replacing `agent-policy` through the public registration seam; turn protocol, tool loop, retry, prose-stop detection, usage accounting, error presentation, cancellation, compaction policy, context assembly, RLM depth counter as configurable snapshot data. |
| Continual harness (`/refine`) | None beyond the record layer. | `/refine` as an ordinary command declaration; validated CRUD + rollback + `record_refinement`; pure-function prompt assembly with local-over-global precedence; markdown skills as context data. |
| Daemon-backed continuity | New crate `pi-rs-daemon` (P5): supervisor owning live session reducers (headless `pi-rs-app` workers running the same Lua package graph — one dispatch engine, not two), single writer per durable record store, the job scheduler (durable wake registration, fake-clock seam), Python kernel subprocess lifetimes, and attachment cursors. Versioned Unix-socket wire (one integer version, capability negotiation, backpressure, clear rejection of incompatible clients). Headless/attach modes in `pi-rs-app` are generic registered roles; single-process operation keeps working. | Background/reattach presentation, session naming, revival policy (including kernel snapshot revival), worker-failure messaging. |
| Recursive subagents + messaging | Minimal and generic: parent linkage in the session record; mailbox routing in the supervisor. A cross-scope signal only if evidence demands it. | Spawn admission owning the policy depth counter (explicit snapshot data, configurable; admission returns a handle immediately, results arrive only via messaging or files); reachability restricted to parent/siblings/direct children as a data table tested as a pure function; messages as durable records delivered on the target's next dispatch; `agent_message`/`agent_observe` as Python skill modules issuing `host_request` frames routed by Lua policy to daemon verbs. |
| Heartbeats, schedules, goals, autonomy | None new — the scheduler ships with the daemon. Rust delivers timer events and enforces ceilings; it does not interpret cron, goals, or autonomy. | Heartbeat/goal/autonomous-mode policy; prompts as data; budget counters checked in the loop; command declarations; a scheduled execution is a normal agent invocation, never a privileged path. |
| Lua-surface extensions (P8) | Tier-2 bindings in `pi-rs-host`, only for capabilities a Prime or dogfood consumer already exercises. | Tier-3 packaged Lua modules for reusable policy helpers. |

There is no hard mechanism ceiling for RLM depth in this tree (the pre-port
`MAX_NEST_DEPTH` does not exist here). The product depth limit is Lua
spawn-admission policy — explicit snapshot data, configurable. A mechanism
bound is added only if a demonstrated failure mode demands one, and then as a
generic host/daemon bound recorded in `DESIGN.md`.

## Doctrine-03 against the faithful port

`DESIGN.md`'s doctrine table defers doctrine-03 (daemon + thin client):
"applies to downstream products, not the compatibility port." That row stays
`deferred` for the parity product — the default composition remains
single-process and Pi-compatible.

The daemon activates doctrine-03 for the **Prime composition** at P5. The
daemon-owned state is named here, once, as the doctrine run-check requires:
**live session reducers, durable record-store writers, the job scheduler,
Python kernel subprocess lifetimes, and attachment cursors.** The wire carries
one integer version; additive changes keep old clients; incompatible clients
get a clear rejection. The run-check — start a turn, kill the client,
reattach, named state intact, in-flight turn completed — is a P5 gate.
`DESIGN.md`'s table gains a Prime-composition note when P5 lands; the parity
contract itself is not amended.

## Phases

### P0 — faithful parity closure

Finish `PLAN.md` exactly as written: A.1–A.3, 9.2–9.11, 8, 10, 11. This plan
adds no work to it; the Prime track (P1/P2) proceeds in parallel per D-F7 and
lands no loop code until the 9.10 agent-policy seam exists. Record the
reference pins above,
including the Prime Agent TypeScript oracle as a hash-locked Nix input. Record
the seed fixture manifest: prose-only stop; one Python execution plus model
continuation; Python state across two executions; Python exception and
timeout; one compaction boundary; one `/refine` mutation; one child/message
lifecycle; daemon detach/reattach; heartbeat, schedule, goal, and
autonomous-mode examples. Each row ends the project as an implemented fixture,
an explicit deferral, or a documented intentional difference.

Gate:

```sh
nix flake check    # final-parity-audit and all PLAN 11 accept criteria green
git tag pi-parity-v0.79.0
```

### P1 — `pi-rs-repl`: persistent Python/IPython bridge (hard mechanism #1)

New crate, created here. Wire protocol: framed, length-prefixed JSON-lines,
one integer version. Host→shim: `execute{id, code}`, `interrupt{id}`,
`host_response{req_id, result}`, `snapshot{}`, `shutdown{}`. Shim→host:
`stream{id, name, chunk}` (bounded chunks), `result{id, ok, value, error}`,
`host_request{req_id, kind, payload}` (mid-execution, from day one),
`snapshot_data{...}` (best-effort revival data). Correlation IDs; late events
from an interrupted or replaced kernel cannot complete a newer request. Reuse
the port's dispatch/effect substrate: bounded channels, scope-owned long-lived
resources with explicit disposal, per-cell watchdog. No secret or host-state
inheritance — env, cwd, and capabilities come from explicit launch data.
Python environment pinned in Nix. Lua seam `pi.repl` as a tier-2 binding. One
throwaway smoke consumer for the gate; the real `python` tool ships at P3.

Parity per layer: REPL-trace over a **real** Python subprocess — recorded cell
scripts asserting event traces: persistence across cells, stdout/stderr
ordering, exceptions, interrupt of `while True: pass`, watchdog kill +
respawn, `host_request` round trip, snapshot/revive, adversarial framing,
stale correlation.

Gate:

```sh
nix build .#pi-rs-repl
nix flake check
nix run .#repl-smoke    # `x = 1` then `x + 1` -> 2 across two frames
```

Plus: `kill -9` mid-execute produces a typed death event within the watchdog
budget and the next execute runs on a fresh kernel; N start/dispose cycles
leave zero surviving Python children.

### P2 — named-record CRUD in `pi-rs-session` (small; parallel with P1)

The record layer described in the placement table. Put/get/delete/list round
trip; latest-value-wins; tombstones hide keys on list; fold respects limits;
corruption reported loudly; kill-during-append leaves the store readable.
Store compaction deferred until a measured problem exists. Contract tests, not
oracle.

Gate:

```sh
nix flake check
```

### P3 — RLM agent loop, ported faithfully (the value)

Lua policy package `prime/rlm/`, loaded as an ordinary package. The agent root
replaces `agent-policy` through the public registration seam — replacement,
not privilege; the default composition never loads it. Introduces the `.#prime`
flake app: the same `pi` binary with a declarative config loading the Prime
package set from the repo — the composition mechanism for all later phases.

**Faithfulness rule.** Read the Prime Agent TypeScript loop in full first;
port its control flow 1:1. Divergences forced by the snapshot/action shape
are listed in this document (or `DESIGN.md` when it gains the Prime section)
with reasons. No improvements and no redesign until trace parity is green.

Owns: turn protocol, tool loop, retry, prose-stop detection, usage accounting,
error presentation, cancellation, declared limits, compaction trigger and
summarization policy, context assembly (system prompt as data, project
instruction discovery, harness block slot filled at P4), the shipped `python`
tool over `pi.repl` (serialized — one kernel, no interleaving), RLM depth as
snapshot data in context.

Parity per layer: scripted-LM + REPL-trace, the headline suite. A scripted
provider registered through the public `pi.register_provider` path replays
fixture scripts of text/tool-call events through the same registration real
providers use — no dedicated test hook. The real loop root dispatches over a
real `pi-rs-repl` kernel; assertions on the full normalized action trace (turn
start, tool start/result, message, prose-stop, compaction, stop reason) and
REPL side effects. Seed corpus: phi's 82 scenario shapes. Where the TS Prime
Agent can be driven deterministically, capture its transcripts and diff shapes
— canonicalize timestamps and IDs; never normalize ordering, stop reasons, or
errors.

Gate:

```sh
nix flake check                          # scripted-LM + REPL-trace >= phi's 82-scenario coverage; parity suites untouched-green
nix build && nix run .#prime -- -p "compute 2**10 in python"    # documented manual smoke, real provider, real kernel
```

First end-to-end usable milestone.

### P4 — continual harness: `/refine`, memories, skills, prompts

Lua policy package `prime/harness/` over the P2 record layer. Schema
collections as data; local vs. global scope as two store paths chosen by Lua
path policy — Rust knows none of these names. `/refine` as an ordinary command
declaration: validated CRUD, rollback, `record_refinement`. Deterministic
prompt projection from local+global entries with local-over-global precedence
— a pure function, fixture-tested. Markdown skills surface into context;
Python-backed skills preload into the kernel through P1's execute path.
Harness state survives restore; the crash-boundary exactly-once rule is stated
here and gated at P5: a partially applied turn resumes or rolls back by one
explicit rule, never duplicating a tool or Python result.

Parity per layer: canonical fixtures — harness state in, rendered prompt block
out; CRUD round trips surviving process restart; scripted-LM scenario where
the model performs a harness op mid-turn; scope/precedence, invalid records,
concurrent-edit conflict.

Gate:

```sh
nix flake check
```

Fixture: fresh session → create memory/skill/prompt note → restart → all
three appear in the injected overview. And: harness package removed → boots,
RLM loop runs with an empty harness.

### P5 — `pi-rs-daemon`: supervisor + versioned wire (hard mechanism #2)

New crate, created here — after the loop and harness because they define what
state the daemon owns; before subagents and autonomy, which stack on it.
Supervisor owns the named state set (doctrine-03 section above). Durable
records are the recovery source after daemon death. Wire: Unix socket,
length-prefixed JSON (same framing idiom as the REPL — one implementation),
one integer wire version; verbs in mechanism-only vocabulary (`attach`,
`detach`, `dispatch`, `subscribe`, `spawn_background`, `list_sessions`,
`schedule`, `unschedule`, `list_jobs`); capability negotiation; local endpoint
auth; backpressure on slow clients; worker death is a supervisor event plus
session-record state, never supervisor death; watchdog on worker handshake and
every dispatch. Thin-client and headless-worker roles in `pi-rs-app` are
generic registered roles; single-process mode keeps working.

Parity per layer: canonical continuity journey plus contract tests — frame
fixtures per verb; version-mismatch rejection; new-client/old-daemon and
old-client/new-daemon; two clients; slow-client backpressure; stale socket;
soak (10 workers, bounded RSS, counted fds).

Gate:

```sh
nix run .#prime-daemon &
nix run .#prime -- attach <id>
nix flake check
```

Green on: the doctrine-03 run-check (start a turn, `kill -9` the client,
reattach, named state intact, in-flight turn completed); daemon crash →
restart → promised durable state recovered without duplicated effects (the
P4 exactly-once rule, tested at each enumerated turn boundary); wire-version
rejection fixture; the one-dispatch-engine proof — the P3 scripted trace
produces an identical action batch in-process and through the socket.

### P6 — recursive subagents + agent-to-agent messaging

Lua policy package `prime/subagents/`, mostly composition — the point of the
sequencing. A child is another instance of the same Lua agent root in a new
scope via `spawn_background` — never a Rust-special agent, no new declaration
kind. Spawn admission owns the policy depth counter, refusing past the
configured product limit with a typed refusal; child budgets, inherited
context/harness visibility, and cancellation propagation are admission-time
data. Messages are durable records addressed by agent id, delivered as events
on the target's next dispatch — store data plus existing event dispatch, not a
new bus; sender identity is the sending scope's provenance. Reachability
(parent/siblings/direct children) is a data table tested as a pure function
over family-tree fixtures. `agent_message`/`agent_observe` are Python skill
modules preloaded into the kernel (P4 mechanism) issuing `host_request` frames
(P1) routed by Lua policy to daemon verbs (P5).

Parity per layer: scripted-LM over the real daemon and real kernels — parent
spawns child, child replies via `agent_message`, parent transcript shows
delivery. Canonical fixtures for mailbox contents, depth-cap refusal,
reachability violation.

Gate:

```sh
nix flake check
```

Green on: parent→child→message round trip; spawn at the depth limit → typed
refusal, not a hang; out-of-family message rejected; disposing the parent
cancels the child's in-flight work while its durable messages persist; mailbox
backpressure; a malicious infinite-loop child cannot hang its parent
(watchdog). One documented manual smoke: real model, spawn-and-reply.

### P7 — heartbeats, schedules, goals, autonomous mode

Lua policy package `prime/autonomy/` over the daemon scheduler. Semantics
pinned from the TS reference: missed-run, overlap, retry, idempotency on
duplicate wakeups, cancellation, time-zone/DST. Goals persist in the harness
store with budget tracking, status injected by the harness prompt projection.
Autonomous mode is bounded self-continuation toward a goal: budget counters
checked in the loop, watchdog-bounded, surfaced as command + status. A
scheduled execution is a normal agent invocation.

Parity per layer: canonical schedule/goal fixtures under the fake clock
(on-time, missed, overlapping, retrying, canceled, DST); scripted-LM autonomy
scenario (3-turn budget → typed budget-exhausted stop);
daemon-restart-between-registration-and-firing fixture.

Gate:

```sh
nix flake check
```

Plus: schedules and goals survive daemon restart and client detach/reattach; a
heartbeat fires into a detached session and the transcript shows the turn on
reattach (documented manual smoke).

### P8 — Lua-surface consolidation + any-harness proof

Third sequencing step. Promote every mechanism Prime actually consumed into
documented `LUA_SURFACE.md` tier-2 rows; package reusable policy helpers as
tier-3 modules; nothing Prime needed may remain private. Build a second,
small, non-Prime harness (a minimal eval-runner or review-loop agent) as a
file-backed package using only the public tiers — the executable evidence that
any other harness can be built in Lua, dogfood-style. No spec-first growth:
this phase documents and proves surface that products already forced into
existence.

Gate:

```sh
nix flake check    # any-harness exerciser + Prime suite + untouched parity suites
```

### P9 — experience sweep, performance contracts, release

- Full user-visible surface polish as Lua data: command tables, configurable
  keybinding tables, themes, status, diagnostics, machine-readable headless
  output. Optional daemon capabilities degrade locally, never blocking attach
  or startup.
- Canonical-experience release suite: one scripted end-to-end journey —
  create session, run Python, `/refine` a memory, spawn and message a child,
  compact, detach, heartbeat fires, reattach, inspect — recorded as the
  regression set; 80×24 + resize + no-color terminal snapshots.
- Skill/package distribution through the public package transport with normal
  provenance/conflict rules.
- Performance by contract: REPL cell round trip, scripted RLM turn latency,
  daemon attach/dispatch round trip, harness fold, kernel cold start, memory
  per idle session/kernel/child. Budgets recorded only after a measured
  release run exists; optimize only failing budgets, preserving trace parity.
- Close the P0 seed fixture manifest: every row implemented, explicitly
  deferred, or a documented intentional difference — the one judge-check,
  reviewed by the user.

Gate:

```sh
nix flake check
nix build .#prime && nix run .#prime -- --machine-readable smoke
```

Release blockers: parity suites green and untouched; TS-oracle transcript
diffs green; kill/restart client preserves daemon-owned state and kill/restart
daemon recovers all promised durable state; a file-backed external package can
replace each Prime root through the public API; no Rust module contains
Prime-specific policy expressible via the snapshot/action interface.

## Dependency spine

P0 faithful parity closure (PLAN.md; tag pi-parity-v0.79.0) — release gate only (D-F7)
P1 pi-rs-repl (Rust, hard #1) ────────────────┐        (P0 ∥ P1 ∥ P2)
P2 record CRUD (Rust, small) ─────────────┐   │        (P1 ∥ P2)
                                            │   │
       P3 RLM loop (Lua, faithful) ◄───────┴───┘   (gated on main's 9.10 seam)
            └─► P4 continual harness (Lua over store)
                 └─► P5 pi-rs-daemon (Rust, hard #2; owns state defined by P3/P4)
                      ├─► P6 subagents + messaging (composition of P1+P4+P5)
                      └─► P7 heartbeats/goals/autonomy (daemon scheduler)
                           └─► P8 Lua-surface consolidation + any-harness proof
                                └─► P9 experience + performance + release
```

## Standing checks (every phase)

- `nix flake check` is the only sentence meaning "tests pass". Completion
  claims cite the Nix commands.
- The parity suites run on every commit and must stay green; a red parity
  check halts Prime work.
- Prime packages load only through the public package/config loader. No
  embedded privilege, no source-name checks.
- Snapshots enter; actions leave. Every new dispatch or effect gets a watchdog
  at introduction. Lua never borrows mutable host state.
- The Pi provider/auth oracle fixtures are never modified.
- Reject at every phase: Rust rewrites of agent decisions; Prime-only Rust
  APIs for Python, records, timers, children, or messages; daemonizing
  unsettled semantics; redesigning the RLM loop while trace parity fails; new
  plugin frameworks beyond the port's public seams; optimization without a
  failing benchmark.

## Decisions

Recorded here, to move into `DESIGN.md` locked-decision rows when the Prime
section lands (`DESIGN.md` is not edited by this plan):

| Decision | Choice | Rationale |
|---|---|---|
| D-F1 Product shape | Prime Agent is a separate composition of ordinary file-backed Lua packages (`prime/` package set + `.#prime` flake app) over the unchanged faithful port; never default-on builtin packs | Additive by construction; the default composition and its parity suites never see Prime sources; no-privileged-path is proven by the loader, not by an ablation shim |
| D-F2 New crates | Exactly two: `pi-rs-repl` and `pi-rs-daemon`, generic and Prime-agnostic, created at their phase | Both are independently testable mechanism boundaries; Rust must not know product vocabulary; empty skeletons are scaffolding nobody exercises |
| D-F3 RLM loop | Lua policy, a replacement agent root through the public registration seam, ported 1:1 from the TS reference; no redesign until trace parity is green | The loop is the product's value; faithfulness is the fastest route to a correct loop |
| D-F4 Doctrine-03 | Activates for the Prime composition at P5 with the named owned-state set; the parity product keeps `deferred` | The daemon is Prime-only product surface; the compatibility port stays single-process |
| D-F5 RLM depth limit | Lua spawn-admission policy — explicit configurable snapshot data; no hard mechanism ceiling is carried over from the pre-port tree; a generic bound is added only with failure evidence | The pre-port `MAX_NEST_DEPTH` does not exist in this tree; least-code until evidence |
| D-F6 Skills | No second tool runtime: the REPL is the model's tool; Python-backed skills preload via the execute path; markdown skills are context data | One execution mechanism, one watchdog story |
| D-F7 Parallel sequencing (2026-08-09) | Prime track starts before the parity gate: branch `prime-agent-faithful` from current `main`; P1/P2 immediately, P3 gated on the 9.10 agent-policy seam; parity suites stay green on every Prime commit (a red check halts Prime work); the parity tag still gates release | User directive to build both tracks concurrently; P1/P2 are additive and isolated, so substrate risk is bounded to merge churn, which continuous branch tracking absorbs. Reverses the original "no overlap in gating" sequencing. Reversed if: a Prime merge reddens a parity suite, or the 9.10 seam lands behind P1/P2 completion |
Flagged for user input before the affected phase:

1. **Prime Agent TS oracle commit.** Which commit pins the value-layer
   behavioral spec? The vendored snapshot in `ref/prime-agent/` is pinned at
   `c22549a` ("fix(ai): use current anthropic abort model (#658)") — confirm
   this is the intended pin, vendored as a hash-locked Nix input per the PLAN
   A.3 oracle rule.
2. **Prime composition identity.** The parity product's identity is locked
   (`pi`, `~/.pi/agent`, `PI_CODING_AGENT_*` overrides). Does the Prime
   composition keep the `pi` binary identity with a distinct state root and
   socket path, or ship a `prime` wrapper identity? Affects P3's `.#prime`
   app and P5's daemon endpoint.
3. **DESIGN.md timing.** When the Prime section and D-F rows land in
   `DESIGN.md` — at P1 start, or with the daemon's doctrine-03 note at P5?
