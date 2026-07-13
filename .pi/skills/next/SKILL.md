---
name: next
description: Continue pi-rs from the first open dependency-ready PLAN.md item, respecting serial gates, explicit waves, focused oracles, and Nix acceptance.
---

# Next PLAN item

Use this skill for one non-orchestrated continuation session. `PLAN.md` and Git
history are the durable handoff; do not infer a different roadmap from the
current implementation.

## 1. Recover the frontier

1. Run `git status --short --branch`, `git rev-parse HEAD`, and
   `git log --oneline -15`. Preserve unrelated user changes; do not begin from
   an unexplained dirty tree.
2. Read `PLAN.md` and `DESIGN.md` in full, then the applicable doctrine files
   under `~/Dev/design/doctrines/`.
3. Inspect the implementation, evidence, and recent diffs for the last closed
   item. A checked box without landed acceptance evidence is plan drift: repair
   or truthfully reopen it before advancing.
4. Scan unchecked PLAN items from top to bottom. The **first open item** defines
   the frontier. It is dependency-ready only when every explicit `depends on`,
   `serial after`, and preceding wave gate is checked and present in merged
   history. Do not skip a blocked or stale first item to work on a later heading.
   Resolve its prerequisite/plan drift or stop with the blocker.
5. If the first item is marked **serial**, it owns the repository frontier: no
   later item starts until all of its acceptance criteria are integrated and the
   box is truthfully closed. If it belongs to an explicit **Wave**, `/next` may
   implement that one first open wave item serially only when no orchestrated
   batch is active; it does not claim or launch sibling wave work.

If the item is too large for one session, choose one acceptance-bearing coherent
slice of that same item. Do not broaden it or start its dependent. Record the
exact remainder in `PLAN.md`.

## 2. Recover only the required contract

- Follow `DESIGN.md`: Rust = generic mechanism; product policy = ordinary Lua
  packages through the public surface; snapshots in/actions out; one declaration
  path per kind; zero-pack boot remains real.
- Read sources on `pi-rust-rewrite` only when they provide a useful mechanism or
  migration clue. They are not product requirements.
- Pi v0.79.0 in `ref/pi` is a focused oracle only when the PLAN item names:
  1. a canonical Pi-feeling frame/input journey, or
  2. provider/model/protocol/auth compatibility.
  Inspect only the source and observation needed for that question. Pi does not
  authorize adjacent product parity.
- Generic Rust mechanisms use invariant/resource-cleanup evidence. Additive Lua
  capability uses an ordinary file-backed package/example. Storage uses the
  XDG/legacy matrix. Performance uses the checked release harness.
- Do not update a Pi-derived fixture from pi-rs output or make normal checks
  depend on an ambient sibling checkout.

## 3. Implement and verify

- Implement only the frontier item/slice and its owned paths. Avoid broad
  formatting, generated churn, compatibility scaffolding, and unrelated cleanup.
- Preserve source neutrality: embedded provenance grants no capability, module,
  lifecycle, priority, or declaration advantage.
- Keep dispatches bounded and async resources cancellable/disposable. Do not
  introduce per-byte/per-cell Lua crossings or unbounded snapshot/history copies.
- Run focused checks while iterating. Use a worktree-local `CARGO_TARGET_DIR` for
  any direct Cargo command.
- Before closing an implementation item, run `cargo fmt --check`, the affected
  focused checks, `cargo test --workspace`, and the PLAN-required Nix check.
  Final build/verification claims are Nix claims; record exact commands and
  outcomes, never checks not run.

## 4. Close the loop

1. Re-read the item's complete acceptance block and inspect `git diff --check`.
2. Update `PLAN.md` in the same change: check the item only if every criterion
   passed; otherwise record the landed slice, exact remaining work, and any
   unrun release/live check.
3. Commit in the established component-prefixed style. Explain the PLAN item,
   chosen mechanism/policy placement, evidence, and checks.
4. End with a concise handoff: commit(s), changed paths, checks/outcomes, exact
   remainder, and the resulting first open dependency-ready item.
