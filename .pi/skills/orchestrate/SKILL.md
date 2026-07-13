---
name: orchestrate
description: Coordinate pi-rs from its first open PLAN item through exclusive serial gates and explicit dependency-ready waves with exact base/path locks and focused evidence.
---

# pi-rs PLAN orchestrator

Act as the sole coordinator and integrator. Never delegate orchestration itself.
Optimize for a green integration branch and safe dependency order, not maximum
worker count.

Modes from the invoking prompt:

- `run` or no mode: recover an active batch, otherwise select, launch, await,
  review, and integrate the frontier batch;
- `status`: observe/report only; do not spawn, integrate, or edit;
- `plan`: print the next valid batch and locks; do not spawn.

Additional user text may narrow scope but may not skip the PLAN frontier.
`${PI_ORCHESTRATE_MAX_WORKERS:-4}` is the hard parallel-worker limit. A serial
gate always has exactly one worker. Ask only when scope is ambiguous, cleanup
would destroy unintegrated work, or a higher limit is requested.

## 1. Recover before creating work

1. Read `PLAN.md`, `DESIGN.md`, `.pi/skills/parallel-plan/SKILL.md`,
   `.pi/skills/next/SKILL.md`, and applicable doctrines in full.
2. Run `git status --short --branch`, `git rev-parse HEAD`,
   `git log --oneline -20`, and `git worktree list --porcelain`.
3. Inspect `${PI_PARALLEL_STATE_ROOT:-$HOME/.local/state/pi-rs-parallel}` and
   `parallel/*` branches. If a current batch exists, use
   `../parallel-plan/monitor.sh --once`; running work is recovered, never
   duplicated. Old branches without current metadata are patch reservoirs, not
   merge-ready assignments.
4. For exited workers, inspect logs, branches, worktrees, commits, and exit codes
   before selecting anything new.
5. The integration worktree must be clean, on a non-`parallel/*` branch, and at
   the ref workers receive as `integration`. Preserve unrelated changes and stop
   on dirt.
6. Record the pinned `ref/pi` revision if present. It is used only for named
   canonical-experience or provider/auth evidence, never as a whole-product
   specification or via an unrelated sibling checkout.

## 2. Compute the exact PLAN frontier

Use this algorithm; do not choose attractive later work:

1. Verify checked items against merged history and cited acceptance evidence. A
   falsely closed item is reopened/repaired before any later work.
2. Scan items top-to-bottom. The first unchecked item is **F**. No item after F
   is eligible except unchecked siblings carrying the same explicit PLAN wave
   marker and readiness gate as F.
3. Resolve every explicit `depends on`, `serial after`, and prose gate against
   checked items **and merged commits**. If F's prerequisite is absent, the
   frontier is blocked/plan-drifted; do not skip it.
4. If F is marked **serial**, `serial after ...`, owns an evolving shared
   contract/hot file, or has no explicit wave marker, choose a **serial gate**.
5. If F carries an explicit **Wave X** marker and PLAN's “After ... may run Wave
   X” gate is satisfied, wave candidates are F plus subsequent unchecked Wave X
   items whose prerequisites are all merged in the same base. Stop at the first
   serial/different-wave boundary. Never invent a wave from apparently disjoint
   code.
6. A dependent serial item is not ready until every required item in the named
   preceding wave is integrated, checked, and green. If a wave needs multiple
   batches because of worker limits, it remains the frontier; refresh all later
   assignments from the new base.

Thus “first open dependency-ready” means F controls progress; it does not mean
searching past blocked work. A wave is the only exception that admits later
siblings alongside F.

## 3. Design one locked batch

A batch is exactly one of:

- **serial gate:** one worker for F or one acceptance-bearing slice of F;
- **parallel wave:** 2–4 workers for coherent explicit-wave deliverables.

A large serial item may land in slices, but its gate stays open until every
acceptance criterion is integrated and checked. A single parallel-safe candidate
is run as a serial gate; never manufacture concurrency or hand the frontier to
`/next`.

A wave assignment is valid only when:

- every required interface and dependency commit is in the integration base;
- it produces a complete coherent deliverable on that base, not speculative
  scaffolding;
- implementation paths are disjoint from every sibling;
- acceptance evidence runs in that worker's isolation;
- it does not depend semantically on a sibling's unmerged decision.

Hot/shared paths—e.g. root manifests, lockfiles, generated indexes,
`flake.nix`, central registries, `main.rs`, `api.rs`, or product root Lua—have
one writer per batch or remain integrator-owned. A “shared-file exception” must
name the path and its sole writer; it never means concurrent edits are safe.
Serialize slices that need the same evolving interface.

For every worker record:

- `slug`;
- `item`;
- `assignment` (one coherent deliverable);
- `base` (exact integration `HEAD`);
- `integration` (current branch/ref);
- `dependencies` (specific commits already merged into base, or `none`);
- `paths` (exclusive paths/globs and named shared-file exceptions).

Every worker in one batch receives the same base. Persist
`kind=serial|parallel`. In `plan` mode, print the frontier reasoning, batch kind,
worker table, locks, focused evidence class, and serial/wave completion criteria,
then stop.

## 4. Assign the correct evidence

Each assignment states one evidence class:

- mechanism invariant/resource cleanup;
- public file-backed Lua capability/ablation;
- named canonical Pi experience;
- exhaustive provider/model/protocol/auth compatibility;
- XDG/read-only-legacy matrix;
- measured release performance.

Direct workers to pinned Pi only for the two Pi-named classes and only for the
specific question. Canonical observations do not authorize neighboring parity.
Provider/auth is exhaustive only within the subsystem boundary in `DESIGN.md`.
`pi-rust-rewrite` may suggest mechanisms but is never a product oracle. Normal
checks consume compact checked fixtures, not ambient repositories.

## 5. Launch without moving the base

Choose a batch ID such as `YYYYMMDD-HHMM-<topic>`. Immediately before launch,
recheck that integration is clean and both `HEAD` and `integration` equal the
batch base. Launch a serial gate once or each parallel worker once:

```sh
../parallel-plan/spawn-worker.sh \
  --wave "$batch" \
  --kind "$kind" \
  --slug "$slug" \
  --item "$item" \
  --base "$base" \
  --integration "$integration" \
  --dependencies "$dependencies" \
  --paths "$paths" \
  --assignment "$assignment"
```

The launcher creates isolated branches/worktrees, copies the skills, links the
canonical pinned reference, assigns a worktree-local Cargo target, records
metadata, disables ambient prompts/extensions/skills, and loads only
`parallel-plan`.

If a parallel launch fails, keep already launched independent workers but record
the failed assignment. If a serial launch fails, the gate remains open and
orchestration stops. Never weaken an assignment to make launch succeed.

## 6. Await without interference

For `run`, wait for every worker in the batch:

```sh
../parallel-plan/monitor.sh --wave "$batch" --wait --interval 30
```

Use `--once` for `status` or diagnosis. Until all workers exit:

- do not change the integration tip—this preserves every worker's base lock;
- do not edit worker trees/logs/state or merge workers into each other;
- do not launch another batch or dependent work;
- treat process exit as a signal to review, not proof of readiness;
- preserve blocked/cancelled worktrees for inspection.

If the session cannot safely keep waiting, report the active batch and leave it
intact for the next `/orchestrate`.

## 7. Review and classify every result

For each exited worker inspect:

- metadata, full log/handoff, and exit code;
- clean status;
- `git log --reverse --format=fuller <base>..<branch>`;
- commit bodies and `git diff --stat <base>...<branch>`;
- changed paths versus ownership;
- dependency provenance, evidence class, checks, and architecture fit.

Classify:

- **ready:** clean, committed, in scope, replayable on base, credible evidence;
- **blocked:** prerequisite/evidence/acceptance missing;
- **stale:** required API/base semantics moved;
- **invalid:** path/scope breach, speculative behavior, contaminated evidence,
  unrelated changes, or wrong oracle.

Exit 0 is necessary but insufficient. A failed process may contain useful
commits, but those require full review and are not relabeled ready for throughput.

## 8. Integrate and satisfy the gate

Only the orchestrator edits integration or `PLAN.md`.

1. Order ready commits by actual dependency, then least shared changes first.
2. Cherry-pick single-purpose commits individually. Resolve only expected
   integrator-owned/shared reconciliation. Abort and reassign any evolving API
   or hot-file semantic conflict from the new base.
3. After each accepted slice, run its focused checks and fail-closed generators.
4. Reconcile manifests/indexes in a dedicated integrator commit.
5. After the batch, run formatting, affected focused suites,
   `cargo test --workspace`, and the PLAN-required Nix checks from integration
   when implementation changed. Docs/fixture-only gates use their explicit
   structural/idempotence checks. Worker results never replace integrated checks.
6. Update `PLAN.md` truthfully and leave integration clean.

A **serial gate closes** only when: terminal handoff reviewed ready; all accepted
commits integrated in order; focused and required integrated checks pass; every
item acceptance criterion is met; PLAN is updated; integration is clean. A
partial/blocked/stale/conflicted/check-failing result leaves the gate open and
no dependent work starts.

A **wave closes** only when every required Wave X deliverable and its acceptance
criteria are integrated, PLAN is truthful, reconciliation is complete, required
integrated checks pass, and the tree is clean. One ready sibling does not unlock
the dependent serial gate. Reassign exact remainder from the current base.

## 9. Continue or report

In `run`, continue to the newly computed frontier only when the previous gate or
wave is completely green and enough context remains for another full
launch/wait/review/integrate cycle. Stop on active workers, unresolved gates,
failed checks, user decisions, or only blocked/stale work.

Report compactly:

- integration base → final tip;
- batch kind/ID and worker classifications;
- serial/wave completion state;
- commits integrated/rejected/blocked/rescheduled;
- exact integrated checks/outcomes;
- active processes/worktrees retained;
- next first open dependency-ready serial gate or explicit wave.
