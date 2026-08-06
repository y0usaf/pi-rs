# Prime Agent on pi-rs — build plan

**Scope.** Rebuild Prime Agent's value layer in Rust+Lua on top of pi-rs, the
same way pi-rs rebuilt pi. pi-rs already owns provider/auth parity (oracle
fixtures, done) and the snapshot/action kernel. This plan adds the entire
Prime Agent delta — RLM loop, persistent Python/IPython REPL, continual
harness, recursive subagents + agent messaging, daemon-backed continuity,
skills, heartbeats/goals/autonomous mode, RLM depth control — as Lua product
policy over a small number of new Rust mechanisms. It does not re-litigate any
settled decision; it sequences and gates the work.

**Source of truth for placement.** pi-rs `DESIGN.md` "Mechanism and policy
boundary" is normative. Rust owns: Lua VM, watchdogs, immutable snapshots,
typed actions/effects, terminal primitives, async OS ops, provider/auth
engines, and a generic durable record store. Lua owns: application/agent/
frontend/session state machines, the tool loop, retry, queue, compaction,
context policy, tools, commands, themes, config, provider selection, sessions.
Every Prime Agent feature below is mapped to exactly one side of that table.

**Naming.** New builtins live under `crates/pi-rs-builtins/<area>/` and are
added to `crates/pi-rs-builtins/default.json` in load order. New mechanism
goes in the crate that already owns that concern (effects → `pi-rs-host`,
records → `pi-rs-session`, provider registry → `pi-rs-ai`). The existing
`pi.*` Lua surface is extended by adding a **versioned** submodule
(`pi.repl.v1`, `pi.daemon.v1`) rather than by growing an existing `v1` in a
breaking way. New declaration kinds are avoided; subagents reuse the existing
root mechanism, not a new `DeclarationKind`.

---

## Phase 0 — ground truth, DESIGN addendum, and harness scaffolding

**Why first.** pi-rs `DESIGN.md` currently marks doctrine 03 (state-owning
daemon, thin client) **deferred** with the explicit note "Detachable/multi-
viewer sessions would trigger a separate daemon + versioned-wire design, not
hidden coupling now." Prime Agent's background continuity is exactly that
trigger. Canon requires DESIGN.md before code, so the daemon and the REPL —
the two decisions that change the locked boundary table — must be written
down as locked decisions before any mechanism is built.

**Scope (no product code yet):**

- Write `DESIGN.prime.md` (or a clearly-marked Prime addendum section in
  `DESIGN.md`) recording, each with a date and a reason:
  - **Locked decision D-P1:** the persistent Python REPL is Rust mechanism.
    It is a long-lived, bidirectional, framed subprocess channel — a new
    effect kind — not the existing one-shot `ProcessRequest`. Placed in
    `pi-rs-host` (async OS effects row). Watchdog-bounded, scope-owned,
    cancellable. *Hot-path guard:* framed request/response crosses once per
    cell execution; never one callback per streamed byte (bounded chunks).
  - **Locked decision D-P2:** the REPL is a *child* of an agent scope, never
    a privileged global. Bare core still boots without it; the REPL is an
    optional Lua tool that acquires a kernel handle through a public seam.
  - **Locked decision D-P3:** the daemon is a new, separate supervisor owning
    the state that must outlive a viewer (background sessions, the durable
    record store's writers, scheduled heartbeats). The interactive `pi`
    process becomes a thin client. Versioned wire protocol (one integer,
    bumped on breaking change; additive = old clients keep working). This
    *activates* doctrine 03 from its deferred state.
  - **Locked decision D-P4:** the RLM agent loop is Lua policy, shipped as a
    replaceable **agent root** at higher priority than `pi.builtins.agent`
    (priority 0). It is not a Rust change and not a fork of the kernel.
  - Record the four required DESIGN.md sections if a new file: Locked
    decisions, Architecture (module map, decision-making vs machinery),
    Deferred, Roadmap with checkable criteria.
- Amend the doctrine-conformance table row for doctrine 03 from `deferred` to
  `follows`, citing D-P3, and note the REPL hot-path guard under the
  functional-core row.
- Add the two new crate skeletons (`pi-rs-repl`, `pi-rs-daemon`) as empty
  library crates wired into the workspace `Cargo.toml` and `flake.nix` so Nix
  builds them from day one. No logic yet.

**Rust vs Lua split:** Rust = two empty crate skeletons + workspace/flake
wiring. Lua = none.

**Parity/test layer:** none (documentation + scaffolding).

**Go/no-go gate (run):**
- `rg '^## (Locked decisions|Architecture|Deferred|Roadmap)' DESIGN.prime.md`
  returns four hits (canon design-doc check).
- `nix build` succeeds with the two new empty crates in the workspace.
- `git grep -n "deferred" DESIGN.md | grep -i daemon` shows the doctrine-03
  row updated to reference D-P3.

---

## Phase 1 — Python/IPython REPL bridge (Rust mechanism, hard piece #1)

**Why first among features.** Every higher feature — RLM loop, subagents,
skills, the model-as-Python-programmer workflow — depends on the model being
able to run Python in a persistent kernel. The brief names this the hardest,
most subtle piece; it is sequenced before everything that stacks on it. It is
pure mechanism and can be built and gated in isolation against a real Python
subprocess with no agent loop present.

**What pi-rs has today.** `crates/pi-rs-host/src/effects/process.rs` implements
`ProcessRequest`/`ProcessStream`: spawn, optional one-shot stdin write, stream
stdout/stderr to exit, then reap with a watchdog timeout and a process-group
kill (`send_signal` / `ProcessGuard`). This is deliberately one-shot and
cannot host a persistent kernel.

**Scope (Rust mechanism):**

- New crate `pi-rs-repl` (mechanism only, no product vocabulary), depending on
  `pi-rs-host` for `CancellationToken`/`ScopeId`/`Control` and on `tokio`.
- A **kernel handle**: owns one long-lived Python (IPython) child process with
  a **framed, length-prefixed JSON-wire protocol** over stdin/stdout. Frame =
  `{id, kind, payload}`; kinds are `execute`, `interrupt`, `complete`,
  `inspect`, `shutdown`, and streaming `stdout`/`stderr`/`result`/`error`
  events. This is a typed host request protocol, exactly the "models write
  Python, typed host requests" shape.
- Reuse the existing effect substrate: bounded channels, `EffectTimeout`,
  `ResourceLease`, scope-owned disposal, process-group kill. The REPL stream
  uses `EffectOptions::long_lived()` (timeout disabled) for the channel but a
  **per-cell watchdog** (bounded) for each `execute` — a runaway cell is
  interrupted, not a hung host.
- Interrupt maps to SIGINT to the child's process group (IPython catches it as
  `KeyboardInterrupt`); the channel survives an interrupt and stays usable.
- Bounded streaming: stdout/stderr cross as bounded chunks/events into a
  bounded channel (`DEFAULT_STREAM_CAPACITY`-style), never one Lua callback
  per byte. Backpressure: a full channel applies backpressure to the reader
  task, not unbounded buffering.
- Cancellation: disposing the owning agent scope kills the child (new process
  group, `SIGKILL` fallback) and fails outstanding cell futures. Reload /
  session replacement disposes the kernel (stale-handle rule).
- New Lua seam `pi.repl.v1`: `start(opts) -> handle`, and handle methods
  `execute(code) -> iterator of events`, `interrupt()`, `complete(code)`,
  `shutdown()`. Installed from `pi-rs-host/src/bindings/` next to
  `effects.rs`, registered only when the REPL crate is present (embedding is
  distribution, not privilege — a file-backed package could provide the same
  seam, but the shipped one is embedded for provenance).

**Scope (Lua policy):** none in this phase beyond a thin smoke tool used only
by the gate (the real `python` tool ships in Phase 3 with the RLM loop).

**Rust vs Lua split:** ~all Rust (`pi-rs-repl` + `pi.repl.v1` binding). Lua =
one throwaway smoke tool for the gate.

**Parity/test layer — scripted-LM + REPL-trace (mechanism half).** Per the
established strategy, the REPL does not oracle-fixture cleanly (it is
interactive/behavioral). Gate it with a **REPL-trace harness**: a Rust
integration test (`crates/pi-rs-repl/tests/`) drives a real `python3`/IPython
subprocess through a recorded script of cells and asserts the returned event
traces (state persistence across cells, stdout capture, exception surfacing,
interrupt of an infinite loop, completion results, clean shutdown, disposal
killing the child). This mirrors phi's proven approach.

**Go/no-go gate (run):**
- `nix flake check` runs `pi-rs-repl` integration tests green against a real
  Python: a two-cell trace where cell 2 reads a variable set in cell 1
  (persistence), a `while True: pass` cell interrupted within the watchdog
  (returns `KeyboardInterrupt`, kernel still alive for a following cell), and
  a syntax-error cell returning a structured `error` event.
- A leak check: start + dispose N kernels, assert no surviving `python` child
  processes (process-group reap) and stable RSS.
- Watchdog proof: one dispatch that blocks on a cell past the per-cell bound
  is interrupted and the host remains responsive.

---

## Phase 2 — durable record store hardening for the harness (Rust mechanism, small)

**Why here.** The continual harness (`/refine`, memories, skills, prompts) and
the daemon both need a durable store that supports not just append-only
session logs but **update/delete of named records** (a memory is revised; a
skill is rolled back). pi-rs's `pi.records.v1` over
`crates/pi-rs-session/src/record_store.rs` is a *versioned append-only JSON
record* store with cursors — sufficient for sessions, but the harness wants
CRUD-by-key semantics. Build the small delta now so the harness (Phase 4) and
daemon (Phase 5) share one store.

**Scope (Rust mechanism):**

- Extend `pi-rs-session`/`record_store.rs` (or add a sibling module) with a
  **named-record layer** on top of the append-only log: a record kind
  `{collection, key, op = put|delete, value}` folded to a latest-value view.
  This stays least-power: the store still stores opaque JSON values; the
  collection/key/CRUD meaning is data in the records, not new store verbs.
- Cursors and bounded windows already exist; add a `fold_collection(name)`
  read that streams the latest-value map within `StoreLimits` (no full-history
  copy per read, per the persistence hot-path guard).
- Keep atomic append, locking, corruption reporting. Deletes are tombstone
  appends (append-only is preserved; compaction/snapshot is a later,
  optional optimization — deferred).
- Expose via `pi.records.v1` additions: `put(collection, key, value)`,
  `get(collection, key)`, `delete(collection, key)`, `list(collection)`.
  Additive to the versioned binding (old Lua keeps working).

**Scope (Lua policy):** none here (the harness schema is Phase 4).

**Rust vs Lua split:** all Rust in `pi-rs-session` + small `pi.records.v1`
additions. Lua = none.

**Parity/test layer — canonical / contract.** The store is a generic
mechanism tested against its own contract (pi-rs evidence category 1), not
against pi. Unit + integration tests in `crates/pi-rs-session/tests/`.

**Go/no-go gate (run):**
- `nix flake check` green on new record-store tests: put/get/delete round-trip,
  latest-value-wins on repeated put, tombstone hides deleted key on `list`,
  fold respects `max_window_records`, corruption in the log is reported not
  silently swallowed, and a kill-during-append leaves the store readable.

---

## Phase 3 — RLM agent loop (Lua policy, port faithfully, the value)

**Why now and why it matters.** The RLM loop is the value. Canon says port it
faithfully first, do not redesign it. It needs the REPL (Phase 1) because the
model's primary tool is the persistent Python kernel. It is built before
subagents, harness, and daemon because those are *inputs to* and *clients of*
the loop, not the other way around.

**Faithful-port rule.** The reference is Prime Agent's RLM loop semantics:
turn protocol, tool loop, prose-stop, compaction, the persistent-REPL-as-tool
convention, and the turn/steer/follow-up/interrupt surface. The pi-rs shipped
agent (`crates/pi-rs-builtins/agent/turn.lua`) is a *starting skeleton*, not
the spec — its `run_turn`/`request`/`settle_tools` structure is the right
shape, and the RLM loop extends it rather than replacing the kernel.

**Scope (Lua policy) — new builtins under `crates/pi-rs-builtins/rlm/`:**

- `rlm/loop.lua` (`pi.rlm.loop@1`): the RLM reducer. Registers an **agent
  root** at priority > 0 so it replaces `pi.builtins.agent` through the public
  `roots.register` seam — no privileged path. Reuses `pi.agent.queue@1`
  (steering/follow-up/interrupt) and `pi.agent.tools@1` (tool declaration)
  unchanged.
- Turn protocol: the system-prompt/tool convention that makes the model write
  Python into the persistent kernel and read back results; the explicit
  turn/continuation contract; prose-stop detection (model stops when it
  answers in prose rather than calling a tool), bounded by declared limits
  (reuse the `limits` pattern from `turn.lua`: `max_requests`,
  `max_tool_iterations`, etc., plus RLM-specific bounds).
- Compaction policy: a `rlm/compact.lua` that decides what enters model
  context and emits compaction through the session package's existing
  compaction-record concept (the session store already names compaction
  records). Keep compaction a Lua reducer over the conversation, never a Rust
  branch.
- `rlm/tools/python.lua`: the shipped `python` tool whose `execute` sends the
  model's code to `pi.repl.v1` and returns bounded output; declared through
  `pi.agent.tools@1` with `serialize = true` (one kernel, calls must not
  interleave) and `owner = "pi.rlm"`.
- Depth control: the loop reads the kernel's nested-dispatch depth (already
  bounded at `MAX_NEST_DEPTH = 8` in `crates/pi-rs-host/src/kernel.rs`) and
  exposes the current RLM depth to the model via the system prompt, refusing
  to spawn past the bound. (Actual subagent spawning is Phase 6; the loop
  here just honors the depth signal.)
- Add the package files to `crates/pi-rs-builtins/default.json` after the
  existing `agent/*` entries (they depend on `pi.agent.*` modules being
  loaded).

**Scope (Rust mechanism):** ideally none. If the loop needs an additional
signal (e.g. a cheap "current dispatch depth" read), expose it read-only via
`pi.kernel.v1` rather than adding a product branch — but only with evidence
batching cannot solve it (per the boundary table's amendment rule).

**Rust vs Lua split:** ~all Lua (`crates/pi-rs-builtins/rlm/`). Rust = at most
a read-only depth accessor.

**Parity/test layer — scripted-LM + REPL-trace (the headline suite).** This is
the suite phi proved (82 passing integration tests driving the real loop
through a scripted LM over a real Python subprocess). Recreate it here:
- Inject a **scripted LM** by registering a test-only provider through the
  existing `register_api_provider` seam (`crates/pi-rs-ai/src/registry/
  api_registry.rs`) — a provider whose stream replays a recorded script of
  `text_delta`/`toolCall` events, driven by a fixture file. This is the same
  public registration path real providers use, so no privileged test hook.
- Drive the real RLM loop root through dispatch with that scripted LM and a
  real Python subprocess; assert the full action trace (`agent_turn_start`,
  `agent_tool_start` for the `python` tool, `agent_tool_result`,
  `agent_message`, prose-stop, compaction fired at the bound) and the REPL
  side effects (state persisted across the model's cells).

**Go/no-go gate (run):**
- `nix flake check` green on the RLM scripted-LM+REPL-trace suite: at minimum
  a multi-turn trace where the model runs Python across turns with state
  carried over, a prose-stop trace (loop terminates without a tool call), a
  tool-iteration-limit trace (loop stops at the bound, emits `agent_error`),
  and a compaction trace.
- Zero-pack / replacement invariant preserved: `nix build` with the `rlm`
  package removed still boots the bare coding agent (`pi.builtins.agent` at
  priority 0 takes over) — proving the RLM loop is a replaceable root, not a
  privileged builtin.

---

## Phase 4 — continual harness: /refine, memories, skills, prompts (Lua policy over the store)

**Why after the loop.** The harness is how the running agent persists and
refines its own prompt/memory/skill layer; it is consumed by the loop (the
system prompt is assembled from it) and it edits the store built in Phase 2.
Building it now lets later phases (subagents, heartbeats) record into it.

**Scope (Lua policy) — `crates/pi-rs-builtins/harness/`:**

- `harness/store.lua` (`pi.harness.store@1`): the schema over the Phase-2
  named-record store. Collections: `memories`, `skills`, `prompts`,
  `subagents`, `refinements`. Each entry = `{key, value, scope = "local"|
  "global", updated_at}`. This is the continual-harness CRUD mapping to
  `put/get/delete/list` — pure Lua policy; the Rust store knows none of these
  names (least-power: the store stays a table of opaque JSON).
- `harness/refine.lua` (`pi.harness.refine@1`): the `/refine` command.
  Registered as a **command declaration** through the existing
  `pi.roots.v1.declare("command", …)` seam (DeclarationKind `command` already
  exists), so it is an ordinary composable declaration, not a hand-wired
  builtin. Implements create/update/delete/rollback of memory/skill/prompt/
  subagent entries and `record_refinement`, writing to the store.
- `harness/assemble.lua`: builds the agent's system prompt / context block
  from the current local+global entries, honoring the local-over-global
  precedence rule. Called by the RLM loop's `configure`/turn start.
- Skill loading: a skill is a markdown (prompt-only) or Python-backed entry
  stored as data; the harness surfaces markdown skills into context and, for
  Python-backed skills, makes their `reference`/`arguments` contract callable
  by the model writing Python in the REPL (the REPL is already the model's
  tool — skills need no new execution mechanism, only registration into
  context). This keeps least-code: no second tool runtime.

**Scope (Rust mechanism):** none beyond Phase 2. (If global-vs-local needs a
second physical store location, that is two store *paths* chosen by Lua path
policy — `pi.config.paths@1` precedent — not new Rust.)

**Rust vs Lua split:** all Lua over `pi.records.v1`. Rust = none.

**Parity/test layer — canonical-experience fixtures.** The harness is not a
provider/auth surface and not the interactive loop, so it takes canonical
fixtures: scripted `/refine` command journeys asserting store contents and the
assembled system-prompt block, plus record-level round-trips.

**Go/no-go gate (run):**
- `nix flake check` green on harness tests: `/refine` create→update→rollback
  of a memory round-trips through the durable store and survives a simulated
  reload (re-open store, entry intact); `list` reflects a delete; the
  assembled context block reflects local-over-global precedence.
- Ablation: remove the `harness` package, `nix build` still boots and the RLM
  loop runs with an empty harness (no privileged coupling).

---

## Phase 5 — daemon: supervisor + versioned wire (Rust mechanism, hard piece #2)

**Why after the loop and harness, before background continuity is claimed.**
The daemon is the second hard mechanism and the one that changes the process
model. It must exist before any feature that outlives a viewer (background
sessions, scheduled heartbeats). It is sequenced after the loop and harness
because those define *what state* the daemon must own (sessions, the harness
store, scheduled jobs) — building the daemon first would mean guessing its
state.

**What pi-rs has today.** Nothing — DESIGN.md defers doctrine 03 and states
"there is no `pi msg` CLI or socket." This phase introduces both, deliberately.

**Scope (Rust mechanism) — new crate `pi-rs-daemon`:**

- **Supervisor/worker split.** The daemon owns: the set of live background
  sessions, the writers for the durable record store (single writer per store
  — clients attach and render), and the scheduler for heartbeats/goals. The
  interactive `pi` process becomes a thin client that attaches, dispatches,
  renders, detaches.
- **Versioned wire protocol.** One integer `DAEMON_WIRE_VERSION`, bumped on
  every breaking change; additive changes keep old clients working; breaking
  changes reject old clients with a clear error, never a silent misparse
  (daemon-thin-client doctrine). Transport: a Unix socket under the XDG state
  dir (`$XDG_STATE_HOME/pi/daemon.sock`), framed length-prefixed JSON (same
  framing idiom as the REPL channel — one implementation, cross-cutting).
- Wire verbs (mechanism only, no product meaning): `attach(session_id)`,
  `detach`, `dispatch(session_id, event) -> action batch`,
  `subscribe(session_id) -> stream of action batches`, `spawn_background(
  spec) -> session_id`, `list_sessions`, `schedule(job)`, `unschedule(id)`,
  `list_jobs`. The daemon applies each dispatch through the same
  snapshot/action kernel and watchdog as the interactive path — there is one
  dispatch engine, not two.
- **State ownership per canon.** The state the daemon owns is named in
  DESIGN.prime.md (D-P3): live session reducers + their record-store writers +
  the job scheduler. The check is: kill the client, restart it, confirm that
  named state is intact.
- The Lua agent/frontend/session packages are unchanged — the daemon hosts
  the *same* Lua VM and package graph in the worker, so policy stays Lua.
  Only the *transport* of events/actions moves onto the socket.

**Scope (Lua policy):** `crates/pi-rs-builtins/daemon/` — session policy that
decides *when* a session goes background vs foreground, naming, and reattach
presentation. Thin; the mechanism is Rust.

**Rust vs Lua split:** mostly Rust (`pi-rs-daemon` + a thin-client mode in
`pi-rs-app/src/launcher.rs` + `pi.daemon.v1` binding). Lua = the
background/foreground session policy.

**Parity/test layer — canonical + contract.** The daemon is generic mechanism
(evidence category 1) plus a canonical continuity journey. Not an oracle
surface.

**Go/no-go gate (run):**
- The doctrine-03 run-check, quoted and passing: start daemon, start a
  background session, detach/kill the client, reattach a new client, and
  confirm the named state (session reducer history + harness store) is intact.
- Wire-version gate: an old-versioned client is rejected with a clear version
  error on a breaking change, and an additive change lets an old client
  attach.
- One-dispatch-engine proof: the same scripted-LM trace from Phase 3 produces
  the identical action batch whether driven through the in-process path or
  through the daemon socket.
- `nix flake check` green.

---

## Phase 6 — recursive subagents + agent-to-agent messaging (Lua policy + minimal mechanism)

**Why after the daemon.** Subagents are separate agent roots with their own
scopes; messaging between them is durable, scope-safe communication. The
daemon (Phase 5) provides the natural rendezvous and lifetime ownership for
sibling/child agents that outlive a single dispatch; the harness (Phase 4)
provides the subagent *specs* (reusable delegation specs stored as data).

**Scope (Lua policy) — `crates/pi-rs-builtins/subagents/`:**

- `subagents/spawn.lua`: spawn a child agent as a **new agent root** in a new
  scope (reuse `pi.kernel.v1` scope/root machinery and the existing
  `MAX_NEST_DEPTH = 8` bound for RLM depth control — no new `DeclarationKind`).
  Admission returns a child handle immediately (id, name, depth); results
  arrive only via messaging or files, never as a spawn return value.
- `subagents/message.lua`: agent-to-agent messaging restricted to parent,
  siblings, and direct children (matching Prime Agent's routing rule). A
  message is a durable record in the harness store (Phase 2/4) addressed by
  agent id, delivered as an event to the target agent root on its next
  dispatch — least-power: messaging is data in the store + the existing event
  dispatch, not a new bus. No spoofing: sender identity is the sending scope's
  provenance.
- `subagents/observe.lua`: read-only family status + bounded recent-message
  previews, reading the same store.
- The RLM loop's depth control (wired in Phase 3) now actually refuses/honors
  spawn requests based on the kernel nested-depth signal.

**Scope (Rust mechanism):** minimal. Reuse the daemon's session/scope
ownership for child lifetime and the store for the mailbox. Only add a Rust
change if a cross-scope *signal* (not data) is required, and then as a generic
scoped event in `pi-rs-host`, with evidence.

**Rust vs Lua split:** ~all Lua over existing kernel/scope/store/daemon seams.
Rust = at most a generic scoped-event signal.

**Parity/test layer — scripted-LM + canonical.** Subagent orchestration is
behavioral: drive a parent + child through scripted LMs (two registered
scripted providers) and assert the spawn/admission/message/deliver/depth-cap
trace. The messaging store contents are canonical fixtures.

**Go/no-go gate (run):**
- `nix flake check` green: a parent agent spawns a child (admission returns a
  handle, not an answer), the child replies via a message delivered on its
  next dispatch, the parent observes it, and a spawn at `MAX_NEST_DEPTH` is
  refused with the depth error.
- Routing rule enforced: an agent cannot message outside parent/siblings/
  direct-children (attempt is rejected).
- Disposal: disposing the parent scope cancels the child's in-flight work
  (scoped disposal), and the child's durable messages persist in the store.

---

## Phase 7 — heartbeats, schedules, goals, autonomous mode (Lua policy over the daemon scheduler)

**Why late.** These are higher-order policies that *drive* the agent on a
schedule or toward a persistent goal, and they need the daemon's scheduler
(Phase 5), the harness's goal/prompt storage (Phase 4), and the loop (Phase
3). They are the least foundational and most policy-heavy, so they come last.

**Scope (Lua policy) — `crates/pi-rs-builtins/autonomy/`:**

- `autonomy/heartbeat.lua`: agent-owned RLM heartbeats — scheduled jobs that
  dispatch a wake event to an agent root through the daemon `schedule` verb.
  Start/stop/list as command declarations.
- `autonomy/goals.lua`: persistent thread goals with budget tracking, stored
  in the harness store; start when asked, mark complete when the objective is
  met. A goal injects its status into the assembled context (harness
  `assemble.lua`).
- `autonomy/autonomous.lua`: autonomous mode — a bounded loop that lets the
  agent self-continue toward a goal without a user turn, bounded by declared
  limits and the watchdog; surfaced as a command + status.

**Scope (Rust mechanism):** none beyond the daemon scheduler. The scheduler
itself (timers firing `schedule`d dispatches) is part of Phase 5.

**Rust vs Lua split:** all Lua. Rust = none (scheduler already built).

**Parity/test layer — canonical + scripted-LM.** Scheduled-fire and
goal-completion journeys are scripted-LM traces; stored goal/schedule state is
canonical fixtures.

**Go/no-go gate (run):**
- `nix flake check` green: a scheduled heartbeat fires a dispatch through the
  daemon at the due time (fake-clock or short-interval) and produces the
  expected action batch; a goal's budget is enforced (loop stops at budget);
  autonomous mode terminates at its declared bound.
- Kill-client persistence: schedules and goals survive a client detach/reattach
  (daemon owns them).

---

## Phase 8 — packages/skills distribution, performance, and the release gate

**Why last.** Distribution polish and the performance contract close the work;
they depend on all features existing.

**Scope:**
- Skill/package distribution: install/list of file-backed skill packages over
  `pi.packages.v1`, with the same provenance/conflict rules as any package
  (no privileged path).
- Performance: extend `tests/performance/` to cover the new paths — REPL
  cell round-trip, a scripted RLM turn, daemon attach/dispatch round trip,
  harness store fold. Add numeric budgets to DESIGN.prime.md **only after** a
  prescribed release measurement exists (per pi-rs's own rule: no unmeasured
  budget claims). The REPL and daemon must not regress the existing startup/
  render/input budgets.
- Zero-pack, per-package, whole-root replacement, stale-handle, watchdog,
  cancellation, cleanup, and XDG matrices re-run with the new packages present
  and removed.

**Parity/test layer:** all three — oracle (provider/auth, unchanged and still
green), scripted-LM+REPL-trace (loop/REPL/subagents), canonical (harness,
daemon continuity, autonomy) — plus performance budgets.

**Go/no-go gate (run):**
- The pi-rs release gate, extended: `nix flake check` + release `nix build` /
  `nix run` on a clean checkout, with the full evidence matrix (mechanism
  invariants, capability/ablation, experience grids, provider/auth fixtures,
  XDG matrices, session policy, RLM/REPL scripted-LM traces, daemon
  continuity, performance budgets) green.

---

## Dependency order at a glance

```
0 DESIGN + scaffolding
   └─► 1 Python REPL (Rust, hard #1)         ─┐
   └─► 2 record-store CRUD (Rust)             │ feeds
        └─► 3 RLM loop (Lua, faithful) ◄──────┘  (loop needs REPL)
             └─► 4 continual harness (Lua over store)   (needs 2,3)
                  └─► 5 daemon (Rust, hard #2)          (needs to know state: 2,3,4)
                       └─► 6 subagents + messaging (Lua + daemon)
                            └─► 7 heartbeats/goals/autonomous (Lua over daemon)
                                 └─► 8 distribution + performance + release gate
```

Two hard Rust mechanisms are sequenced **before** their dependents: the REPL
(1) before the RLM loop (3); the daemon (5) before subagents (6) and autonomy
(7). The record store (2) precedes both the harness (4) and the daemon (5).

## Parity/test strategy per layer (summary)

| Layer | Strategy | Where it lives |
|---|---|---|
| Provider/auth (already done) | **Oracle** differential fixtures vs pinned Pi v0.79.0 | `tests/*-parity` (keep green, untouched) |
| Python REPL bridge | **Scripted-LM + REPL-trace** vs real Python subprocess | `crates/pi-rs-repl/tests/` |
| RLM agent loop | **Scripted-LM + REPL-trace**: scripted provider via `register_api_provider`, real loop, real REPL, assert action trace | new suite under `crates/pi-rs-builtins` tests / host integration |
| Continual harness | **Canonical-experience** fixtures over the durable store | harness tests |
| Daemon | **Canonical** continuity journey + contract (wire version, one dispatch engine) | `crates/pi-rs-daemon/tests/` |
| Subagents/messaging | **Scripted-LM** (multi-provider) + **canonical** mailbox fixtures | subagent tests |
| Autonomy | **Scripted-LM** + **canonical** schedule/goal fixtures | autonomy tests |
| Experience/performance | unchanged **canonical** grids + **budgets** (no unmeasured claims) | `tests/experience`, `tests/performance` |

## Explicitly deferred (with reasons)

- **Record-store compaction/snapshotting** of the CRUD layer — tombstone
  appends are correct first; compact only on a measured size/latency problem.
- **A second tool runtime for skills** — the REPL is the model's tool; skills
  register into context, they do not get their own executor (least-code).
- **New `DeclarationKind` for subagents** — subagents reuse the existing root
  mechanism; a new kind is added only with evidence the root seam cannot
  express it.
- **Rust changes to the RLM loop** — the loop is Lua; a Rust signal is added
  only with evidence batching cannot solve a measured hot path.
- **Cross-machine daemon transport** — Unix socket under XDG state first; a
  network transport is a separate versioned-wire decision if ever wanted.

## Canon cross-check

- **least-code:** the delta is concentrated in two mechanisms (REPL, daemon)
  plus Lua policy; every later feature reuses the record store, the root seam,
  and the REPL rather than building new machinery.
- **least-power:** harness CRUD and messaging are *data* in the store; subagent
  specs are *data*; the store keeps opaque JSON values.
- **no-privileged-path:** RLM loop is a higher-priority agent root; `/refine`
  and autonomy are command declarations; the REPL seam `pi.repl.v1` is
  installable by a file-backed package; zero-pack/ablation gates prove it.
- **functional-core/imperative-shell:** every dispatch (in-process or through
  the daemon) is watchdog-bounded; the REPL has a per-cell watchdog.
- **daemon-thin-client:** Phase 5 activates doctrine 03 with a named owned
  state set and a versioned wire; the client kill/reattach check is the gate.
- **nix-verify:** every gate above is a `nix flake check` / `nix build` /
  `nix run` invocation; no claim is made on a non-Nix command.
