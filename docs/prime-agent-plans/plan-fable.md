# Prime Agent in Rust on pi-rs — Build Plan (Fable)

Status: plan. Decisions inherited from pi-rs and the shared brief are treated as settled.
Repo shape assumed throughout: a new repo **`prime-rs`**, started from pi-rs at a pinned
commit (mirroring how Prime Agent forked pi), keeping the eight pi-rs crates
(`pi-rs-ai`, `pi-rs-ai-auth`, `pi-rs-ai-types`, `pi-rs-app`, `pi-rs-builtins`,
`pi-rs-host`, `pi-rs-session`, `pi-rs-tui`) and adding exactly two new mechanism crates:

- **`prime-repl`** — persistent Python/IPython bridge (Rust mechanism, Phase 1)
- **`prime-daemon`** — supervisor/worker daemon + wire protocol (Rust mechanism, Phase 4)

Everything else in the Prime Agent delta is Lua product, shipped as file-backed packages
under `lua/prime/` loaded through the same public package loader as any third-party
package (`[[canon:no-privileged-path]]`).

Two hard Rust-mechanism pieces, called out up front and sequenced before what stacks on them:

1. **Python/REPL bridge** (`prime-repl`) — Phase 1. The RLM loop (Phase 2), skills-as-
   Python-modules (Phase 3), and subagent messaging skills (Phase 5) all execute through it.
2. **Daemon** (`prime-daemon`) — Phase 4. Subagent spawn/messaging (Phase 5), background
   continuity, heartbeats/schedules (Phase 6) all stack on it. pi-rs has no daemon today.

Parity strategy per layer (fixed, per brief):

| Layer | Strategy |
|---|---|
| Provider/auth | Oracle parity vs pinned Pi v0.79.0 — already done in pi-rs; keep green, never touch |
| RLM loop + Python REPL | Scripted-LM + REPL-trace parity: real loop, real Python subprocess, scripted model. phi proved the shape (82 tests); port the *approach*, not phi code |
| Harness, subagents, daemon, TUI surface | Canonical-experience fixtures: golden transcripts / session records / rendered frames |

---

## Phase 0 — DESIGN.md, repo, and the bare-core gate

**Scope.** No features. Establish the repo, the design record, and CI before any code
(`[[canon:design-doc]]`).

- Fork pi-rs → `prime-rs` at a pinned commit; record the pin in DESIGN.md.
- Write `DESIGN.md` with the four required sections. Locked decisions must name, as
  sentences other rules' checks can find:
  - the extension boundary module (the `roots.v1.dispatch` seam in `pi-rs-host`) —
    for the functional-core grep check;
  - the state the daemon will own (session records + agent process lifetimes + message
    queues) — for the daemon-thin-client kill/restart check, even though the daemon is Phase 4;
  - the two new crates and why each is mechanism, not policy;
  - the parity table above;
  - explicitly: "the RLM loop is ported faithfully from Prime Agent TS; redesign is out
    of scope until parity is green."
- CI (Nix): `nix flake check` runs the existing pi-rs suite; add the
  no-privileged-path job now (build with `lua/prime/` and builtins removed; confirm bare
  core boots and does what DESIGN.md says a bare build does) so it exists before there is
  anything to privilege.

**Rust vs Lua.** Neither; docs + flake + CI wiring only.

**Gate (go/no-go).**
- `rg '^## (Locked decisions|Architecture|Deferred|Roadmap)' DESIGN.md` → 4 hits.
- `nix flake check` exits 0 on the fork (all inherited pi-rs tests, incl. oracle
  provider/auth fixtures, still pass).
- Bare-core CI job exits 0.

---

## Phase 1 — `prime-repl`: the persistent Python/IPython bridge (hard mechanism #1)

**Scope.** The model's primary tool is "write Python into a persistent kernel." Build the
mechanism only; no agent loop yet.

**Rust mechanism (`prime-repl` crate).**
- Spawn a Python subprocess running a small vendored shim (`prime_repl_shim.py`, shipped
  as data in the crate) that embeds `IPython.core.interactiveshell.InteractiveShell`.
  Do **not** speak the full Jupyter/ZMQ protocol — the shim + JSON-lines over stdio is the
  smaller thing (`[[canon:least-code]]`; rejected: ipykernel/ZMQ, which drags in a
  message spec, heartbeat channels, and HMAC we don't need).
- Typed JSON-lines protocol, versioned with one integer, with these frames:
  - host→shim: `execute{id, code}`, `interrupt{id}`, `host_response{req_id, result}`,
    `snapshot{}`, `shutdown{}`
  - shim→host: `stream{id, name, chunk}`, `result{id, ok, value, error}`,
    `host_request{req_id, kind, payload}` (emitted **mid-execution** when Python code
    awaits an agent capability — this is the subtle part and must be in the protocol from
    day one), `snapshot_data{...}` (best-effort pickle of revivable names, for session
    revival).
- Watchdog (`[[canon:functional-core]]`): per-execute wall-clock budget; on breach send
  SIGINT, escalate to SIGKILL + respawn; the kernel death is a typed event, never a hang.
- Expose to Lua through the existing effect seam in `pi-rs-host`: Lua emits typed actions
  (`repl.execute`, `repl.interrupt`, `repl.respond_host_request`), Rust applies them and
  delivers typed events into the next snapshot. Lua never holds a process handle.
- Output discipline: byte-capped stream buffers with typed truncation markers (the TS
  agent depends on bounded tool output; make the cap mechanism-level).

**Lua policy.** A thin `prime.repl` package that registers the tool declaration and
formats results — same public API a stranger's tool would use.

**Parity/tests: REPL-trace parity.**
- Golden traces: a corpus of (code in → frames out) recordings covering: persistent state
  across executes, stdout/stderr interleaving, exceptions, `%%bash` cells, interrupt,
  watchdog kill + respawn, mid-execution `host_request` round-trip, snapshot/revive.
- Traces are recorded once against the shim itself and reviewed by hand against Prime
  Agent (TS) behavior for the semantically load-bearing cases (state persistence,
  truncation shape, error rendering); they then pin behavior forever.
- Property: kill -9 the kernel mid-execute → host receives typed death event within the
  watchdog budget, next execute works on a fresh kernel.

**Gate.** `nix build .#prime-repl && nix flake check` green including the trace corpus and
the kill/respawn property test. A REPL smoke binary (`nix run .#repl-smoke`) executes
`x = 1` then `x + 1` → `2` across two frames, proving persistence.

---

## Phase 2 — RLM agent loop, ported faithfully (the value; Lua policy)

**Scope.** Port Prime Agent's agent loop as a Lua **agent root** replacing/parameterizing
pi-rs's stock agent root: turn protocol, tool loop, retry, queueing, prose-stop
semantics, compaction trigger + summarization policy, context assembly (system prompt
composition incl. project AGENTS.md discovery), RLM depth accounting (the counter and
its plumbing, even though recursion arrives in Phase 5).

**Faithfulness rule.** Read the TS loop in full first; port control flow 1:1 into
`lua/prime/agent/`. Divergences (there will be some — snapshot/action shape differs from
TS mutation) are listed in DESIGN.md Deferred/Locked-decisions, each with a reason. No
"improvements" until the parity suite below is green (`[[canon:amend]]`-adjacent
discipline: the loop is right; don't redesign it).

**Rust mechanism.** Ideally none. If the port exposes a missing primitive (e.g., a timer
or cancellation seam), add it to `pi-rs-host` as a generic effect, never as an
RLM-specific hook. Budget: small.

**Lua policy.** `lua/prime/agent/` (loop, turn, compaction, context), `lua/prime/tools/`
(repl tool from Phase 1 as the primary tool), `lua/prime/prompts/` (system prompt as data
— tables/templates, rung 2–3 on the least-power ladder, not string-building code).

**Parity/tests: scripted-LM + REPL-trace (the phi-proven harness).**
- Build the harness first: a scripted provider registered through `pi-rs-ai`'s public
  provider interface (no test backdoor) that replays a fixed assistant-turn script; the
  loop runs for real over a **real** `prime-repl` subprocess; assertions are on the full
  transcript (messages, tool frames, stop reasons, compaction events).
- Port the *scenarios* of phi's 82-test integration suite as the seed corpus: multi-turn
  tool loops, error turns, retry, prose-stop, interrupt mid-tool, compaction firing at
  the threshold, context assembly.
- Where Prime Agent (TS) can be driven deterministically, capture its transcripts for the
  same scripts and diff shapes (not byte-for-byte — canonicalize timestamps/ids).

**Gate.** `nix flake check` green with the scripted-LM suite ≥ the phi scenario count for
the loop core; one end-to-end `nix run .#prime -- -p "compute 2**10 in python"` against a
real provider completes a full turn through the real kernel (manual, documented in
DESIGN.md as the smoke ritual). Bare-core job still green (the agent root loads as a
package, not a privilege).

---

## Phase 3 — Continual harness: `/refine`, memories, skills, prompt notes (Lua policy)

**Scope.** The persistence layer of the value-add: harness state (prompt notes, memories,
skill entries, subagent specs), local vs global scoping, `/refine` command, harness
overview injection into the system prompt, skills-as-Python-modules preloaded into the
kernel.

**Rust mechanism.** None new. Use `pi-rs-session`'s generic durable record store; harness
records are just typed records with a schema version integer. If the record store lacks
a needed primitive (secondary listing by kind, scoped namespaces for local/global), add
it generically there.

**Lua policy.** `lua/prime/harness/`: record schemas (tables — rung 2), CRUD exposed both
as commands (`/refine`) and as REPL-visible operations; prompt assembly reads harness
state from the snapshot and renders the compact-summary block. Skill entries that are
Python modules get installed into the kernel via Phase 1's `execute`/preload path — no
new protocol frames.

**Parity/tests: canonical-experience fixtures.**
- Golden fixtures: harness state in → rendered system-prompt block out (pure function,
  rung 4 — trivially fixture-testable).
- Record-store round-trip: create/update/delete each entry kind, restart the app,
  overview matches.
- Scripted-LM scenario: model calls a harness CRUD op mid-turn; transcript fixture pins it.

**Gate.** `nix flake check` green; fixture: fresh session → create memory/skill/prompt
note → restart → all three appear in the injected overview. Bare-core job green (harness
is a removable package).

---

## Phase 4 — `prime-daemon`: supervisor/worker daemon (hard mechanism #2)

**Scope.** Background continuity: sessions that outlive their viewer. This is the second
hard mechanism and must land **before** subagents/messaging (Phase 5) and scheduling
(Phase 6), which stack on it.

**Rust mechanism (`prime-daemon` crate).**
- Supervisor process owning: the session registry, per-agent worker processes (each
  worker = a headless `pi-rs-app` instance running the Phase 2 agent root), and the
  message queues between agents.
- Wire protocol over a unix socket: JSON-lines, **one integer version bumped on every
  wire change** (`[[canon:daemon-thin-client]]`); additive changes keep old clients,
  breaking changes reject with a clear error. Commands: attach, detach, list, spawn,
  send-message, observe, kill. Events: session output frames, lifecycle, message-delivery.
- Client mode: the existing `pi-rs-tui` frontend gains an attach transport — render
  frames from the socket instead of the in-process app. Attach/detach must be pure
  viewer operations.
- Crash containment: worker death is a supervisor event and a session record state, never
  supervisor death. Watchdog on worker handshake.

**Lua policy.** Session policy over the record store decides what continuity *means*
(what is persisted per turn, what a resumed session revives, incl. Phase 1's kernel
snapshot revival). The daemon persists and supervises; Lua decides content. Daemonized
operation is optional policy — the single-process mode from Phases 1–3 keeps working.

**Parity/tests: canonical-experience + the canon check.**
- Protocol fixtures: recorded frame sequences per command, replayed against the
  supervisor; version-mismatch fixture (old client vs bumped version → clean rejection,
  never a misparse).
- The daemon-thin-client run-check as an automated test: start daemon, start a turn,
  `kill -9` the client, reattach, assert the DESIGN.md-named state (session transcript,
  agent liveness, queued messages) is intact and the in-flight turn completed.
- Soak: 10 concurrent workers, supervisor RSS bounded, no fd leaks (counted, not vibes).

**Gate.** `nix run .#prime-daemon` + `nix run .#prime -- attach <id>`; kill/reattach test
green in CI; protocol-version reject fixture green; `nix flake check` green.

---

## Phase 5 — Recursive subagents + agent-to-agent messaging

**Scope.** `rlm('sub-task')` spawn semantics, parent/sibling/child reachability rules,
`agent_message`/`agent_observe` as Python skill modules, RLM depth limits, result-by-
message (never by return value).

**Rust mechanism.** Small: a `spawn_agent` daemon command (worker with parent linkage
recorded in the session record) and message routing in the supervisor. Both are generic
daemon capabilities — nothing RLM-specific in Rust.

**Lua policy + Python skills.** `lua/prime/subagents/`: spawn policy (compose child
prompt, depth accounting, admission handle back to the model), family-roster rules
(parent/siblings/direct children only; deeper relays through intermediates) as a **table
of reachability rules** (rung 2), message-to-prompt injection policy. The
`agent_message`/`agent_observe` Python modules preloaded into the kernel (Phase 3
mechanism) issue `host_request` frames (Phase 1 mechanism) that Lua policy routes to
daemon commands (Phase 4 mechanism). This phase is mostly composition — which is the
point of the sequencing.

**Parity/tests: scripted-LM for behavior, canonical for wire.**
- Scripted-LM scenario: parent script spawns child (scripted too), child replies via
  `agent_message`, parent transcript shows the delivered message. Runs over the real
  daemon + real kernels.
- Reachability table tested as a pure function against a fixture of family trees.
- Depth-limit fixture: spawn at max depth → typed refusal, not a hang.

**Gate.** The scripted parent/child round-trip passes under `nix flake check`; a manual
smoke (real model, `await rlm('say hi and reply')`) documented and performed once.

---

## Phase 6 — Heartbeats, schedules, goals, autonomous mode, compaction polish

**Scope.** The scheduling/autonomy layer: heartbeat timers waking daemon sessions, goal
records with budgets, autonomous-mode loop policy, `/compact`-from-REPL, RLM depth
config surface.

**Rust mechanism.** One generic primitive: a durable timer/schedule facility in
`prime-daemon` (fire event into a session at time T / every interval), stored in the
record store. Everything it *does* on fire is Lua.

**Lua policy.** `lua/prime/schedule/`, `lua/prime/goal/`: heartbeat prompts as data,
goal state machine, autonomy guardrails (budget counters checked in the loop), the
`compact` and `goal` skill surfaces.

**Parity/tests: canonical + scripted-LM.**
- Timer fixture with a fake clock injected at the mechanism seam: schedule → fire →
  session receives the typed wake event.
- Scripted-LM autonomy scenario: goal with a 3-turn budget → loop stops with typed
  budget-exhausted stop reason.
- Restart fixture: schedules survive daemon restart (they live in the record store).

**Gate.** All above green under `nix flake check`; manual smoke: heartbeat fires into a
detached session and the transcript shows the turn on reattach.

---

## Phase 7 — Full parity sweep, packaging, distribution

**Scope.** Close the loop on the compatibility promises and ship.

- Provider/auth oracle suite: re-run and confirm untouched-green (the one exhaustive
  compatibility promise; it has been in CI since Phase 0 — this is the sign-off, not the
  first run).
- Canonical-experience sweep: a curated end-to-end fixture set covering the whole surface
  (onboarding, /refine, subagent round-trip, daemon reattach, heartbeat) recorded as the
  release regression suite.
- Performance by contract: benchmark the dispatch path and REPL round-trip; record
  budgets in DESIGN.md; `nix run .#bench` compares against them
  (`[[canon:unix]]`: work, then measure — benchmarks exist before any optimization lands).
- Packaging: `nix run github:y0usaf/prime-rs` works cold; the no-privileged-path CI job
  (bare core, no `lua/prime/`, no builtins) is a release blocker.
- Docs: README + DESIGN.md updated; every divergence from Prime Agent TS behavior listed.

**Gate.** `nix flake check` green across all suites; cold `nix run` demo script passes;
bench within recorded budgets; DESIGN.md divergence list reviewed by the user (the one
judge-check that cannot be automated).

---

## Dependency graph (why this order)

```
P0 design/CI
 └─ P1 prime-repl (mechanism #1)
     └─ P2 RLM loop (Lua, the value — earliest point it can be faithful AND real)
         ├─ P3 continual harness (record store only; no daemon needed)
         └─ P4 prime-daemon (mechanism #2)
             ├─ P5 subagents + messaging (composes P1+P3+P4)
             └─ P6 heartbeats/goals/autonomy (daemon timers)
                 └─ P7 sweep + ship
```

P3 and P4 are parallelizable if there are two hands; sequenced here for one.

## Standing checks (every phase, not a phase)

- `nix flake check` is the only sentence that means "tests pass" (`[[canon:nix-verify]]`).
- Bare-core/no-privileged-path CI job runs on every commit from Phase 0 on.
- Functional-core grep: the DESIGN.md-named boundary module has zero `&mut`-host-state
  hits; runs in CI as a text check.
- Every new dispatch/effect gets a watchdog at introduction, not retrofitted.
- Provider/auth oracle fixtures never modified; a red oracle test halts the phase.
