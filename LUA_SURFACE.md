# Public Lua surface

pi-rs has exactly three public Lua surface tiers. The tiers classify compatibility and delivery; they do not create different trust or privilege levels. A capability is available only when its owning inventory row is `implemented`—listing a target here does not claim unfinished PLAN work is complete.

## 1. Pi-compatible API

The **Pi-compatible API** is the Lua translation of Pi v0.79.0's public extension contract: `ExtensionAPI`, event and context values, UI operations, registration behavior, discovery rules, and other extension-visible outcomes. Lua uses idiomatic spellings where documented (for example, `registerTool` → `register_tool`) while preserving Pi behavior.

`EXTENSION_INVENTORY.md` is the closed inventory for this tier. Its source-derived rows and Pi differential fixtures establish compatibility; they do not inventory additive pi-rs mechanisms or reusable Lua libraries.

## 2. Additive mechanism API

The **additive mechanism API** is pi-rs's Lua-native host capability superset. It exposes mechanisms needed to construct the shipped product and the maintained dogfood packages when Pi's extension API or ambient Node runtime does not provide a suitable Lua contract. Examples include process, filesystem, network, crypto, terminal, agent, session, cancellation, and lifecycle primitives.

These APIs promise their documented Lua contract, not Node module emulation or Pi product behavior. Each addition needs an owner in the construction or external-capability inventory plus a file-backed exerciser, translated Pi example, or dogfood consumer. Additive mechanisms may not change the default Pi-compatible product.

### Low-level capability register (PLAN 9.9)

Lua-native mechanisms owned by the construction/dogfood rows, exposed on the `pi` table. Each owns only its external resource: opaque handles kill/close/abort/drop the resource on disposal, never mutable product state, and embedded/file-backed capabilities are identical. Exercisers live under `examples/extensions/*.lua` and their host tests under `crates/pi-rs-host/tests/*_bindings.rs` / `crates/pi-rs-app/tests/`:

- `pi.process.spawn(cmd, args?, opts?) -> handle` — managed subprocess with stdio pipes; `handle:read_stdout/read_stderr/write_stdin/wait/kill/dispose`; process-tree SIGTERM/SIGKILL (`opts.signal` aborts it); Drop kills the tree so no process survives disposal. `pi.process.kill(pid)` ignores `pid == 0` (which would signal the caller's whole process group), so a targeted kill never widens into a group SIGTERM.
- `pi.tcp.connect(host, port, opts?) -> handle` — framed TCP client; `handle:write/read/read_line/close/is_closed`; Drop closes the socket.
- `pi.fs` extended — `readlink/symlink/lstat/chmod/rename/unlink/access/copy_file/mkdtemp/remove_dir/remove_dir_all`, atomic `write_file_atomic`, richer `stat` (mode/uid/gid/nlink), and `watch_file(path, cb) -> handle` (pollable; `handle:poll/close`; disposal stops the watcher thread). `chmod` accepts octal perms/sticky bits but refuses setuid/setgid modes (privilege-escalation shape).
- `pi.crypto` / `pi.buffer` — `sha1/sha256/md5`, `xxhash32/xxhash64`, `random_uuid`, `base64_encode/decode`, `byte_length`.
- `pi.http.fetch(url, opts?)` and `pi.http.stream(url, opts?, on_chunk)` — abort-aware streaming (`opts.signal` cancels mid-stream); bytes delivered as binary-safe Lua strings.
- `pi.set_timeout/set_interval/clear_timeout/clear_interval` — dispatch-scoped background coroutine timers; clearing prevents firing and pending timers drop with their dispatch.
- Packaged modules — `pi.module.require("pi.tools.file-mutation", "1")` exposes the per-file mutation queue (ownership lease/release, leak-free on error) reused by the builtin and file-backed mutating tools.

## 3. Packaged Lua modules

**Packaged Lua modules** are versioned, reusable Lua libraries distributed with builtin or user packages through the public module/dependency mechanism owned by PLAN 9.7. They hold composable Lua policy and helpers—such as tool factories, session/compaction helpers, and rendering utilities—rather than adding hidden host powers.

A module may use the Pi-compatible and additive APIs, but any host capability it needs must already be public in one of those API tiers. Embedded and file-backed packages resolve the same declared module graph. Chunk-local helpers, concatenation-order globals, and undeclared cross-pack globals are not packaged modules and do not count as public authoring surface.

## No embedded/private tier

There is no embedded/private tier. `include_str!`, a synthetic `<pack:…>` source key, or builtin-package membership records provenance only. It must not change API-table members, module visibility, declaration semantics, precedence, snapshots/actions, watchdog treatment, or runtime/session/dispatch lifecycle.

Consequences:

- builtin policy may use only the same three tiers available to ordinary file-backed packages;
- source-name checks cannot unlock capabilities or bypass public declarations;
- internal Rust functions, host registries, and chunk-local Lua helpers are implementation details, not a fourth authoring tier;
- a builtin-only helper must become a packaged Lua module or remain an open construction-inventory defect;
- any current embedded/file-backed difference is unfinished PLAN work, not an API promise.

## Inventory terminology

Keep inventory claims distinct:

| Inventory | What it classifies | Surface tier evidence |
|---|---|---|
| Pi compatibility | Pinned Pi extension members, events, contexts, UI, loader rules, and examples | Tier 1 compatibility |
| First-party construction | Every shipped policy unit and Rust launch/composition seam | Public declaration/module use across tiers 1–3; no private bypass |
| External-extension capability | Pi API use plus package, process, network, filesystem, crypto, timer, lifetime, and concrete-class needs from the pinned dogfood suite | Tier 1 uses and tier 2/3 requirements |

Construction and capability rows may reference Pi-compatible API members, but must not duplicate the member-level compatibility inventory. Packaged modules are distribution and reuse contracts, not evidence that their underlying host mechanisms are implemented.
