---
name: parallel-plan
description: Implement one coordinator-issued, dependency-ready, path-owned PLAN.md slice from an exact integration base in an isolated pi-rs worktree.
---

# Parallel PLAN worker

You are one worker in a coordinated serial gate or explicit PLAN wave. Optimize
for safe replay into the integration branch, not worker count.

The invoking message must provide all fields unambiguously:

- `item=<PLAN.md item>`
- `assignment=<one coherent deliverable>`
- `base=<exact integration commit>`
- `integration=<integration branch/ref>`
- `dependencies=<required merged commits, or none>`
- `paths=<exclusive paths/globs; every shared-file exception named explicitly>`

Reject an absent/ambiguous field. Do not choose your own item, prerequisites,
path ownership, or adjacent work.

## 1. Verify isolation and locks

1. Run `git status --short --branch`, `git rev-parse --show-toplevel`,
   `git rev-parse HEAD`, `git worktree list --porcelain`, and
   `git log --oneline -15`.
2. The branch must start with `parallel/`, the worktree must be clean, and `base`
   must be an ancestor of `HEAD`. A fresh worker requires `HEAD == base`; a
   resumed worker may be ahead only by commits for this exact assignment.
3. At startup, `integration` must resolve exactly to `base`. Any movement makes
   the assignment stale; stop for a refreshed base. Never merge, rebase,
   cherry-pick, or silently reinterpret the assignment.
4. Every listed dependency must resolve and be an ancestor of `HEAD`. If absent,
   stop blocked. Do not downgrade implementation into speculative fixtures,
   inventory, docs, or adapters.
5. Export a target owned by this worktree for every Cargo-backed command, e.g.
   `CARGO_TARGET_DIR="$PWD/target/parallel-plan"`. Never share another
   worktree's target or artifacts.
6. Never modify another worktree or coordination state outside this worktree.
   Do not edit `PLAN.md` unless `paths` explicitly grants it for integration;
   workers normally leave checkbox/slice closure to the integrator.

## 2. Recover the scoped contract

Read in full:

- `PLAN.md`;
- `DESIGN.md`;
- `.pi/skills/next/SKILL.md`;
- applicable doctrine files under `~/Dev/design/doctrines/`.

Inspect the assigned item's acceptance block, relevant merged diffs, current
implementation, and existing evidence before designing changes.

Oracle selection is explicit:

- generic mechanisms → local invariants, bounds, and cleanup tests;
- public Lua capability → ordinary file-backed example/package;
- named canonical experience → the smallest relevant observation from pinned Pi
  v0.79.0;
- provider/model/protocol/auth compatibility → pinned Pi subsystem sources and
  deterministic focused fixtures;
- storage/performance → the XDG matrix / checked benchmark harness.

Only the canonical-experience and provider/auth categories require `ref/pi`.
When required, verify it is present at the project-pinned revision with
`git -C ref/pi rev-parse HEAD`; never substitute an ambient sibling checkout.
`pi-rust-rewrite` may supply a mechanism clue but is not a product oracle. A
focused Pi question never licenses porting adjacent behavior.

## 3. Enforce frontier, dependency, and path ownership

- Trust only interfaces and dependencies already merged into `base`. If the
  deliverable needs another worker's unmerged API or semantic decision, stop as
  not parallel-safe.
- Implement exactly the assignment. A serial-gate slice remains serial; a wave
  marker permits only the coordinator-assigned sibling work, not later PLAN
  headings.
- Treat `paths` as a hard write boundary. Before touching anything else, stop
  and request expansion. A shared manifest, lockfile, registry, generated index,
  or hot implementation file must be explicitly assigned and may have only one
  writer in the batch; otherwise the integrator owns reconciliation.
- Do not manufacture duplicate registries/adapters to avoid a dependency. Avoid
  broad renames, formatting churn, generated rewrites, and unrelated cleanup.
- Preserve architecture: Rust mechanism; independently replaceable Lua product
  policy through public modules/declarations; immutable snapshots and queued
  actions; bounded dispatch; source-neutral builtins; one declaration path;
  useful zero-pack boot.

## 4. Build acceptance-bearing evidence

- Keep each permanent test responsible for one contract. Do not weaken existing
  tests or regenerate Pi-derived expectations from pi-rs output.
- Pi-derived evidence is allowed only for the assigned canonical experience or
  provider/auth question. Copy reviewed provenance into compact checked fixtures
  so normal checks remain offline.
- Public capability must be exercised file-backed unless the assignment names a
  translated pinned example or dogfood package as the consumer.
- Inventory/generator output must be byte-idempotent and fail closed for missing,
  stale, duplicate, unknown, or unclassified rows.
- Performance claims require the prescribed release harness and environment;
  implementation language is not evidence.

## 5. Shape commits for replay

- Keep commits single-purpose and dependency-ordered. Independently useful large
  fixtures may precede runtime changes; do not mix independent audits with work
  requiring an unmerged interface.
- Prefer additive modules and focused call sites over hot-file rewrites, while
  honoring the actual architecture.
- Before committing, compare both `git diff --name-only base...HEAD` and working
  tree paths against `paths`; remove unrelated changes and run
  `git diff --check`.
- Use the established component-prefixed subject. Each commit body must record:
  PLAN item; exact assignment; original base; landed behavior/evidence;
  dependencies and remaining integration work; exact checks run.

## 6. Verify in isolation

Run focused checks and `cargo fmt --check` while iterating when Rust is affected,
always with this worktree's `CARGO_TARGET_DIR`. Before claiming an implementation
slice complete, run the item's required workspace/Nix checks. For docs-only or
fixture-only work, run the applicable structural/idempotence/link checks rather
than unrelated compilation unless the PLAN item explicitly requires it.

Record exact commands and outcomes. Missing prerequisites, absent oracle data,
cross-worktree artifacts, or another worker's successful run are not acceptance.

## 7. Recheck drift and hand off

At completion, resolve `integration` again. Do not rebase or merge. If it no
longer equals `base`, identify likely path/API conflicts and mark the result
stale pending integrator review; same-wave drift must not occur before all
workers exit.

Finish with:

- branch, original base, and current integration tip;
- commits in required application order;
- changed paths and named shared-file exceptions;
- exact checks/outcomes and isolated target path;
- expected textual/semantic conflicts;
- merged dependencies relied upon;
- exact remaining work;
- status = ready, blocked, or stale.

Do not mark the PLAN item complete.
