# pi.kernel — the kernel composition surface, exposed to Lua

Status: **DESIGN-ONLY — ADDITIVE**. This document locks no behavior; it
specifies the additive `pi.kernel` surface, the staged migration that re-homes
pi-rs's own subsystems onto it, the frozen-surface line it must not cross, and
the re-entrancy/ordering risks with their guards.

Read with: `Design` — `DESIGN.md:9-13` (three commitments), `DESIGN.md:60-73`
(diff 5: everything first-party replaceable Lua), `DESIGN.md:74-79` (diff 6:
private/additive mechanism superset), and the spatiotemporal row
`DESIGN.md:187`.

The parity contract is absolute and additive-only for the *product*: the
shipped composition must stay byte-identical to Pi. `pi.kernel` is a tier-2
additive mechanism (`LUA_SURFACE.md`, tier 2) — a capability Pi lacks, added
on the `pi` table, exercised by a file-backed consumer, that never changes
default `pi.*` behavior. Nothing in this document edits the frozen surface
non-additively; where a tempting re-home would, it is marked **forbidden**.

---

## A. `pi.kernel` SURFACE (additive on the existing `pi` table)

`pi.kernel` is installed in `crates/pi-rs-host/src/api.rs` after the existing
members, exactly one more `pi.set("kernel", …)` beside `pi.on`
(`api.rs:2007`), `pi.spawn` (`api.rs:2132`), `pi.session`
(`session.rs:462`). It does **not** alter `pi.on`/`pi.events`/`pi.spawn`/
`pi.session` semantics — those are frozen (`LUA_SURFACE.md` tier 1). The
bridge it composes is the same host-owned kernel `Context` the daemon and the
TUI already use (`pi-rs-kernel/src/lib.rs:83`).

**Ownership rule (non-negotiable): there is exactly one `Context`, and it is
VM-resident.** `pi.kernel` owns the one kernel `Context` on the Lua
thread (`lib.rs:83`). The `DaemonBoundary` (`crates/pi-rs-host/src/daemon.rs:70`)
keeps only its lifetime/drain ordering; it no longer owns a private `Context`
(Stage 1 folds it in). Therefore mount/attach effects and every `set`/`remove`
run on that single thread through one write path (`lib.rs:111`), and a Lua
`on_change` always fires on that same thread (`lib.rs:165-189`) — no
cross-thread Lua and no second context. Marshalling away from the Lua thread
is a special case guarded in §D.

```lua
-- mount: a component with reads + reversible effects + optional reaction.
-- effects are declarative pairs; each stores the prior snapshot and returns
-- an inverse so pi.kernel.unmount replays them in reverse (lib.rs:151).
pi.kernel.mount{
  reads   = { "theme", "session:leaf" },
  effects = { { key = "editor",  value = "idle" } },   -- snapshot-inverse
  on_change = function(changed_key) re_render(changed_key) end, -- spatial
} -> number | nil  -- scope id

pi.kernel.unmount(id)             -- residue-free (replays inverse reverse)
pi.kernel.get(id)   -> value|nil  -- typed read (lib.rs:100)
pi.kernel.has(key)  -> boolean    -- (lib.rs:103)
pi.kernel.set(key, value)         -- single committed write path (lib.rs:111)
pi.kernel.remove(key)             -- (lib.rs:117)
```

### Read-scope ↔ daemon/session boundary

These mirror `DaemonBoundary::attach/attach_with/unmount/drain/set/get`
(`daemon.rs:119/125/136/144`), not the daemon's Rust signatures — the
surface is Lua-first.

```
pi.kernel.attach(reads, on_change?) -> id   -- read-scope (daemon.rs:119,125)
pi.kernel.detach(id)                       -- unregister reads, no host state
pi.kernel.boundary.list()                  -- mounted scopes + their reads
pi.kernel.boundary.keys()  -> {key -> present}
pi.kernel.boundary.drain()          -- unmount all in reverse registration
```

Boundary keys `daemon:host:vm`, `daemon:session:active` are re-exported
(`daemon.rs:53-56`) so Lua names the same context keys the daemon's two
authoritative mounts project into `pi.kernel` on one substrate.

### Reversible event subscriptions

The kernel's "per-registration-grain spirit" (one subscription = one
reversible registration; `tui/src/lifecycle.rs` `subscribe` and the footer's
`on_branch_change`, which returns a dispose fn) is surfaced as
`pi.kernel.subscribe`:

```
local unsub = pi.kernel.subscribe("pi.kernel.changed", fn)  -- returns unsub fn
unsub()                                                     -- reversible, no residue
```

Events stay bus-only: `pi.kernel.subscribe` seats a callback on the existing
event list (`registry.bus_listeners`, `api.rs:1357`) and returns a dispose
closure that clears exactly its slot; it never changes bus emission order
(`vm.rs:184 emit` — frozen).

### Lifecycle

- **host** mounts a kernel component: the product mounts the VM manager
  (`DaemonBoundary::mount_host` semantics, `daemon.rs:80`) as a `pi.kernel` mount
  under `daemon:host:vm` (Stage 1).
- **viewer** attaches a read-scope: `pi.kernel.attach` over the same `Context`
  (Stage 2's interactive frontend registers as a kernel mount).
- **unmount both**: `pi.kernel.unmount(host)` stops the VM thread (its inverse
  calls `Host::stop`, `daemon.rs:88`); `pi.kernel.detach(viewer)` unregisters
  the reads; residue diff must be empty.

The whole narrative is just the SP-TR ceremony already run in Rust —
`crates/pi-rs-app/tests/spatiotemporal_ceremony.rs` — expressed from Lua, so
it is proven by the same executable ceremony, not a new claim.

---

## B. MIGRATION STAGING

Every stage is additive and grain-sized: it changes only `crates/pi-rs-host`
(plus the one new exerciser file under `tests/`), and it must keep the
**frozen parity suites green** (see §C). The units that must **not** change are
listed per stage. Green-check = the executable command that must stay green.

### Stage 0 — `pi.kernel` surface lands (addition), existing VM silent

Changes: `crates/pi-rs-host/src/api.rs` (one `pi.set("kernel",…)` near
`api.rs:3740`), `crates/pi-rs-host/src/vm.rs` nothing, `crates/pi-rs-kernel`
nothing. The daemon boundary is **not yet exposed**; `pi.kernel` operates on
an empty, self-owned context. Add the exerciser
`crates/pi-rs-app/tests/kernel_lua_surface.rs` + `examples/extensions/kernel-demo.lua`
(file-backed consumer, per the exerciser rule `DESIGN.md`).
Unchanged / forbidden here: `pi.on`/`spawn`/`session` shapes, the builtin
packs, event emission order, session bytes.

Green gate: `cargo test --workspace` (bare-boot a pass) +
`final-parity-audit` (`tests/final-parity-audit`) + the new exerciser. Default
product byte-identical.

### Stage 1 — host lifecycle / session manager re-mounted through the kernel from Lua

Changes: `crates/pi-rs-app/src/main.rs` — the daemon **exposes** its two
authoritative mounts to the VM (handed as a `KernelBridge` at `api.rs` install)
instead of a Rust-only `daemon.mount_host(&host)` at `main.rs:792`; the
`DaemonBoundary` surrenders its own `Context` and the composition (a Lua
fragment inside the always-loaded `agent-core`) **mounts** `daemon:host:vm`
and `daemon:session:active` via `pi.kernel` on the single VM context. This
is the collapse to one substrate: the existing `DaemonBoundary` operations
(`daemon.rs:80,103,119,136`) are now the same kernel, exercised by the same
ceremony, now Lua-driven.

Forbidden here: changing the pi.* shapes, the manifest pack registry order
(`crates/pi-rs-app/src/builtins/manifest.rs:41-76`), or session BYTES. The
daemon still creates the `SessionManager` exactly as today.

Green gate: `cargo test -p pi-rs-app --test spatiotemporal_ceremony` (the
existing precise `daemonBoundary` residue/spatial ceremony — a red there
halts the stage — plus `cargo test --workspace`).

### Stage 2 — agent loop / tools / frontend as their own kernel mounts (composed in Lua)

The interactive frontend is already a Lua pack (`interactive` in
`INTERACTIVE_PACK`, `builtins/mod.rs`). This stage turns its **registration** —
its `pi.*` registrations (`pi.register_tool`, `pi.register_command`,
`pi.register_message_renderer`, the role declarations `decl.rs:626:668`) —
into kernel mounts: the pack calls `pi.kernel.mount{reads=…, effects=…}` for
each unit's lifecycle, so tool/loop/frontend registration is a reversible,
replaceable kernel component composed in Lua, instead of free-standing
registrations. Data flows through `pi.kernel.set` on the declared keys; the
rendered frame produced by `ScopedComponent`/`TuiHost` (`tui/src/lifecycle.rs`)
is unchanged.

Change: `crates/pi-rs-app/src/builtins/interactive/**`, `crates/pi-rs-app/
src/builtins/coding-agent.lua`, tools (`tools/*.lua`) — **Lua only** — plus a
file-backed exerciser for any new `pi.*` member a re-home needs. Forbidden:
any change to `pi.on/events/spawn/session` semantics, the shipped packs bytes
or registry order, and the rendered cells (the frame is a parity oracle).

Green gate: `cargo test -p pi-rs-app --test composability`
`--test assembly` `--test ablation_bare_boot` + the `dogfood-*` suites
(`tests/dogfood-parity`, `tests/dogfood-suite`, `tests/dogfood-translations`)
+ `nix flake check`. A red parity frame direction here halts Stage 2.

### Stage 3 — the multiplexer as THE proof (Lua-mounted, multi-session)

A new **file-backed Lua pack** `examples/extensions/mux.lua` (a real user
perimeter, not builtin) mounts MULTIPLE kernel session components
(`pi.kernel.mount` for each open session), a sidebar chrome that reads
`session:*`/theme keys, and attach/detach of the viewer — all through the
same `pi.kernel`. No new Rust composition path exists; the whole mux is a
thread of `pi.kernel.mount/attach/unmount`.

Green gate: a new app test `crates/pi-rs-app/tests/mux.rs` that loads
`examples/extensions/mux.lua` through the public path and asserts two-session
mount + viewer attach/detach residue-empty (same diff discipline as the SP-TR
ceremony) + the parity suites still green (`cargo test --workspace`,
`nix flake check` — bare-boot + `final-parity-audit`).

`Forbidden` across every stage: altering `pi.*` public shapes, shipped packs
(manifest.rs:41-76), event order, session BYTES, the MEMORY modes surface.
Any of those is an additive-path violation: the additive routing is a new
`pi.kernel` member or key on the same table, never a change.

---

## C. FROZEN SURFACE — the additive line that holds

The design above is additive-only. Confirmed unchanged:

1. **Shipped packs.** The manifest is five declared packages
   (`crates/pi-rs-app/src/builtins/manifest.rs:41-76`: `agent-core`,
   `agent-policy`, `coding-tools`, `print-application`,
   `interactive-frontend`). `pi.kernel` does not add/edit/remove any of them;
   Stage 2 only changes *pack source inside builtin dir*, through the public
   registration seam, never the registry/order. Note: the source currently
   declares five (the task memo said “six” — the correction is recorded; none
   is touched).
2. **`pi.*` public shapes.** `pi.on`, `pi.events`, `pi.spawn`, `pi.session`,
   `register_tool`/… surfaces (`LUA_SURFACE.md` tier 1) are untouched;
   `pi.kernel` is a new table member and only adds.
3. **Event ordering.** Emission is registration-order sequential
   (`vm.rs`, `emit`), one failing handler isolated. `pi.kernel.subscribe`
   keeps its bus slot and never reorders; reverse by id.
4. **Session bytes.** `pi_rs_session` append log / session files are the data
   compatibility contract (`DESIGN.md:139`); `pi.kernel` only points at an
   already-resolved `SessionManagerHandle` (`daemon.rs:103`), never serializes
   it itself.

§A,B above therefore *hole* the line: nothing changes existing shapes, order,
or persisted bytes; every new capability lands under `pi.kernel`.

---

## D. RISKS and guards

1. **Re-entrant notification — dropped `set`.** `Context::notify` guards with
   a reentrancy flag (`lib.rs:91,165-168`): a `set` issued *inside* an
   `on_change` is dropped, not queued. Guard: a handler must never depend on a
   nested `set` delivering; call `pi.kernel.set` at the *root* of the handler,
   or queue through the composition — the surface preserves the same kernel
   behavior, so parity is untouched.
2. **Notification ordering.** Declared readers of the same key fire in scope
   id (mount) order (`lib.rs:170-189`); a lazy reader of a session view depends
   on daemon republish. Guard: keep the order — viewers must attach in the
   order their renders need; never reorder across a `boundary` marshal.
3. **Cross-thread marshalling of a VM-owned event to a kernel-ledger event
   **within a session-active window.** If one `pi.on` subscription unwires and
   a `pi.kernel.subscribe`/`mount` re-seats it while a session turn is live,
   delivery must stay on the VM thread and in registration order; a miss lands
   `on_change` on the wrong thread or reorders the frame. **Guard:**
   `pi.kernel` is the only writer (the VM thread): the daemon never `set`s
directly — it marshals its authority snapshot onto the owned context (Stage 1
primitive), and any re-event-in-window is delivered on the same thread before
the next viewer tick.
4. **Unmount residue.** Inverses replay in reverse registration order
   (`lib.rs:151`); a `mux` that mounts sessions A,B then detaches its viewer
   and unmounts must diff to empty — verified by the ceremony test
   (`§B Stage 3`), never waived.

---

*Scope: docs-only, additive, parity byte-identical. Nothing here edits
`crates/*/src` beyond `api.rs`'s additive `pi.set("kernel",…)` nor the frozen
products; every staging gate is the named `cargo test`/`nix` check staying
green.*