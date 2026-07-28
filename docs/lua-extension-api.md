# Lua mechanism API

The inherited Pi-compatibility extension API is not part of pi-rs. Ordinary Lua
packages receive one compact, versioned mechanism table; embedded provenance
adds no members or privileges.

The complete top-level surface after PLAN 4.1's record slice is:

- `pi.kernel.v1`: package transaction primitives;
- `pi.roots.v1`: application/agent/frontend root facade;
- `pi.terminal.v1`: batched terminal input + retained display submission;
- `pi.models.v1`: catalog lookup + bounded provider event streaming;
- `pi.effects.v1`: bounded filesystem/path/environment, process, timer, and
  cancellation effects;
- `pi.records.v1`: durable append-only record stores at Lua-chosen destinations.

No top-level event bus, command/tool registry, runtime/session/config/settings,
trust/auth UI, `pi.ai`, `pi.tui`, `pi.fs`, `pi.exec`, or `pi.http` compatibility
member is installed. Product declarations and richer capabilities return only
with a demonstrated file-backed consumer.

## Package transaction

A package is a Lua chunk receiving `pi` as `...`. File, memory, and embedded
sources use the same loader, watchdog, publication transaction, scope ownership,
and disposal path.

`pi.kernel.v1` provides:

- `root(definition)`, `declare(kind, definition)`, and `registered(kind)`;
- `action(kind, payload)` and `effect(kind, payload)` queues;
- exact-version `module.define`, `module.require`, and `module.list`;
- immutable `read_handle(value)`, dispatch `cancellation()`, and scoped
  `resource(disposer)`.

Actions/effects publish only after a successful dispatch. Snapshots contain
`version`, `generation`, `scope`, immutable `event`, and immutable `context`.

`pi.roots.v1.register(definition)` accepts `kind = "application" | "agent" |
"frontend"`; `action`, `cancellation`, and `module` are the same canonical
kernel operations.

## Terminal

`pi.terminal.v1.input_buffer()` returns a bounded decoder with `feed`, `flush`,
`clear`, and `buffer`. `display([limits])` returns a retained display handle with
`submit`, `revision`, and `reset_presentation`. Submit complete versioned trees
using `display_schema_version`; no callback occurs per byte or cell.

## Models

`pi.models.v1.find(provider, id)` returns a catalog model or `nil`.
`stream(model, context, options, on_event)` streams complete provider events and
returns the final message. `options.max_events` defaults to 256 and is limited to
`1..=1024`; transport cancellation uses `options.signal`.

## Effects

`pi.effects.v1.fs.read(path[, max_bytes])` and `write(path, contents)` are capped
at 8 MiB. `exists(path)`, `stat(path)` (`type`, `size`, `modified_ms`),
`list(path[, max_entries])`, `make_directory(path)` (recursive), and
`remove_file(path)` cover path metadata; listings default to 1024 entries and
are limited to `1..=16384` (`default_max_entries`, `max_entries`). `stat` on a
missing path raises; `exists` does not.

`pi.effects.v1.path` is pure POSIX path arithmetic — `join`, `normalize`,
`dirname`, `basename`, `extname`, `is_absolute`, `resolve`, `relative`, and
`separator`. It computes locations; it never chooses one.

`pi.effects.v1.env` is an immutable snapshot of the process environment taken
when the host starts: `get(name)` returns one value or `nil`, and `names()`
returns the sorted variable names. Values never cross in bulk, the snapshot
cannot be written, and no variable has a meaning in Rust — XDG/legacy
precedence, credential variables, and defaults are Lua policy. Embedders may
supply an explicit environment through `HostConfig::environment`.

`process.run(program[, args][, options])` caps timeout at five minutes
and output at 8 MiB. `timer.sleep(milliseconds[, signal])` is scope-owned.
`cancellation.new()` returns a signal with `abort`, `is_aborted`, and `wait`.
All queued work is cancelled and settled before package disposal completes.

## Records

`pi.records.v1` persists opaque JSON records; it never interprets a schema and
never chooses a path. Callers pass explicit destinations, normally derived from
the immutable `snapshot.context` storage data, so XDG/legacy policy stays in
Lua.

- `create{directory, name[, limits][, cancellation]}` and
  `open{path[, limits][, cancellation]}` return a store holding an exclusive
  file lock;
- `list{directory[, limits][, cancellation]}` returns `stores` plus explicit
  `diagnostics` (`locked`, `header`, `corruption`, `partial-write`, `io`), so a
  locked or damaged file is reported rather than silently omitted;
- `api_version`, `format_version`, `extension`, and `default_limits`
  (`max_record_bytes` 1 MiB, `max_window_records` 256, `max_window_bytes`
  4 MiB) describe the mechanism.

A store provides `path`, `record_count`, `append(value[, options])` returning
the record sequence, `cursor()`, `copy{directory, name[, record_count]}` for an
atomic prefix snapshot, `close()`, and `closed()`. A cursor provides
`next_sequence()` and `next([{max_records, max_bytes, cancellation}])`, whose
window carries `records`, `start_sequence`, `next_sequence`, `encoded_bytes`,
and `done`. Windows are bounded by the store limits, so iteration never copies
an unbounded history.

Every open store is registered on the owning package or dispatch scope through
the same path as `pi.kernel.v1.resource`: disposing the package closes the store
and releases its lock without waiting for Lua garbage collection. Operations are
synchronous and observe the innermost dispatch cancellation unless an explicit
`cancellation` is passed; an already-cancelled token fails the call before any
blocking work.
