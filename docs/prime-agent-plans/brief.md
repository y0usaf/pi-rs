# Shared brief: Build Prime Agent in Rust on pi-rs (planning task)

You are one of an ensemble of planner subagents working for a single user. Below is the complete established context. Produce a PLAN for building **Prime Agent in Rust on top of pi-rs**. Do NOT re-litigate the decisions; they are settled. Your job is sequencing, scope, and checkable gates.

## Established context (do not relitigate)

### The goal
Take **Prime Agent** (TypeScript, a fork of the `pi` coding agent) and rebuild it in Rust the same way the user already rebuilt `pi` in Rust as **pi-rs**. pi-rs = Rust mechanism kernel + product authored entirely in Lua 5.4 policy. Embedding is distribution, not privilege; no privileged builtins; bare core boots; performance benchmarked by contract; provider/auth parity is the one exhaustive compatibility promise.

### The relationship of the projects
- **pi-rs** (`github.com/y0usaf/pi-rs`): Rust mechanism + Lua product. ~59k Rust LOC (8 crates: pi-rs-ai, pi-rs-ai-auth, pi-rs-ai-types, pi-rs-app, pi-rs-builtins, pi-rs-host, pi-rs-session, pi-rs-tui) + ~8.5k Lua product. Provider/auth parity with pinned Pi v0.79.0 is done (oracle fixtures per protocol). Walking-skeleton example works (application/agent/frontend/middleware roots via `roots.v1.dispatch`, immutable snapshots + typed actions). All tests pass. Single-commit repo, `main`.
- **Prime Agent**: TypeScript, built on pi. Adds the value layer on top of pi: RLM agent loop, continual harness `/refine`, recursive subagents, agent-to-agent messaging, persistent Python/IPython REPL as the model tool, daemon-backed background sessions/continuity, packages/skills, heartbeats/schedules/goals, autonomous mode, compaction, RLM depth control.
- **phi**: the user's earlier Rust rewrite of pi/Prime Agent — their engineering predecessor/learning project. Source of lessons: don't build the product in Rust; don't redesign what's already right. phi is NOT the base and NOT re-used as code; pi-rs is the base.

### Verified: what pi-rs currently LACKS (the Prime Agent delta to build)
Verified by inspecting the pi-rs source (grep):
- No Python/REPL bridge, no IPython kernel, no subagent/subcall/recursion, no RLM loop, no daemon, no continual harness/refine, no skills layers.
So the entire Prime Agent value-add must be added on top of pi-rs.

### pi-rs architecture doctrine (must honor)
- Rust owns MECHANISM: Lua VM & package loading, watchdogs, immutable snapshots, typed action/effect application, terminal/display primitives, async OS ops, provider/auth engines, generic durable record store.
- Lua owns PRODUCT: application/agent/frontend/session state machines, tool-loop, retry, queue, compaction, context policy, tools, commands, themes, config, provider selection, sessions.
- Snapshots in, actions out. No privileged builtins: shipped defaults use same public modules/declarations as file-backed packages. Bare core boots. Persistent sessions are optional Lua policy over the generic record store.

### Prime Agent features to port (the delta), and their likely placement
| Feature | Likely placement | Effort |
|---|---|---|
| RLM agent loop (turn protocol, tool loop, compaction, prose-stop) | Lua policy (agent is replaceable root) | large |
| Persistent Python/IPython REPL (models write Python in a persistent kernel, typed host requests) | Rust mechanism (subprocess, typed protocol, watchdog) | hard, most subtle |
| Continual harness `/refine`, skills, memories, prompts | Lua policy over Rust durable record store | medium |
| Recursive subagents + agent-to-agent messaging | Lua policy (roots/subagents) + some mechanism |
| Daemon-backed background continuity | Rust mechanism (supervisor/worker) + Lua session policy | hard; pi-rs has no daemon today |

### Parity / testing strategy (established)
- Provider/auth = oracle parity (done in pi-rs; keep it).
- RLM loop + Python REPL = scripted-LM + REPL-trace parity (phi proved this works: phi's integration suite drives the real loop through a scripted LM over a real Python subprocess, 82 passing tests). Does NOT oracle-fixture cleanly because it's interactive/behavioral.
- Everything else = canonical-experience fixtures.

### Engineering principles (canon) that must hold
- least-code, least-power, no-privileged-path, functional-core/imperative-shell (every dispatch under a watchdog), daemon-thin-client (state outlives viewer), nix-verify (verification through Nix). DESIGN.md before code. The Agent loop is the value: port it faithfully first, don't redesign it.

## Your deliverable
Write a comprehensive, sequenced, checkable build plan as MARKDOWN to the file given in your task. Requirements:
1. Ordered phases by dependency, each with: scope, Rust-mechanism vs Lua-policy work, and a concrete go/no-go acceptance gate (something you can actually run/verify).
2. Call out the two hard Rust-mechanism pieces (Python/REPL bridge; daemon) explicitly and sequence them before what stacks on them.
3. A parity/test strategy per layer (oracle / scripted-LM+REPL-trace / canonical).
4. Keep the RLM loop port faithful (value it), placed early/appropriately.
5. Pragmatic, ordered, not idealistic. A human could start phase 1 immediately.

Write the plan to disk and reply to your parent with a short summary (not the whole file). Be concrete and specific — cite pi-rs crate names and file placement where possible.
